use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::adapters::csv::load_candles_from_csv;
use crate::adapters::json::load_events_from_json;
use crate::domain::errors::BacktestError;
use crate::domain::types::MarketDataSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketDataMode {
    Candles,
    Events,
    PreferCandles,
    PreferEvents,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketDataRequest {
    pub symbol: String,
    pub venue: Option<String>,
    pub timeframe: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    #[serde(default = "default_prefer_candles")]
    pub mode: MarketDataMode,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl MarketDataRequest {
    pub fn validate(&self) -> Result<(), BacktestError> {
        if self.symbol.trim().is_empty() {
            return Err(BacktestError::InvalidConfig(
                "market data request symbol cannot be empty".to_string(),
            ));
        }
        if let (Some(start), Some(end)) = (self.start, self.end) {
            if start > end {
                return Err(BacktestError::InvalidConfig(
                    "market data request start cannot be after end".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataSourceCapabilities {
    pub supports_candles: bool,
    pub supports_events: bool,
    pub supports_range_filtering: bool,
    pub supports_sequence_guarantees: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceDataFormat {
    Memory,
    Csv,
    Json,
    Parquet,
    RustProvider,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFieldType {
    Timestamp,
    Float64,
    UInt64,
    Utf8,
    Boolean,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceField {
    pub name: String,
    pub data_type: SourceFieldType,
    #[serde(default)]
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSchema {
    pub format: SourceDataFormat,
    pub fields: Vec<SourceField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataSourceMetadata {
    pub provider_name: String,
    pub instrument: String,
    pub timeframe: Option<String>,
    pub data_mode: String,
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub source_schema: Option<SourceSchema>,
    #[serde(default)]
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoadedMarketData {
    pub dataset: MarketDataSet,
    pub metadata: DataSourceMetadata,
}

impl LoadedMarketData {
    pub fn validate(
        &self,
        request: &MarketDataRequest,
        capabilities: &DataSourceCapabilities,
    ) -> Result<(), BacktestError> {
        request.validate()?;
        if self.metadata.instrument != request.symbol {
            return Err(BacktestError::InvalidData(format!(
                "provider returned instrument '{}' but request asked for '{}'",
                self.metadata.instrument, request.symbol
            )));
        }
        if let Some(timeframe) = &request.timeframe {
            if let Some(returned) = &self.metadata.timeframe {
                if returned != timeframe {
                    return Err(BacktestError::InvalidData(format!(
                        "provider returned timeframe '{}' but request asked for '{}'",
                        returned, timeframe
                    )));
                }
            }
        }
        validate_market_data_mode(&self.dataset, &request.mode, capabilities)?;
        validate_market_data_set(&self.dataset, capabilities)?;
        if let Some(schema) = &self.metadata.source_schema {
            super::parquet::validate_schema_for_dataset(schema, &self.dataset)?;
        }
        Ok(())
    }
}

pub trait HistoricalDataSource {
    fn name(&self) -> &str;
    fn capabilities(&self) -> DataSourceCapabilities;
    fn load(&self, request: &MarketDataRequest) -> Result<LoadedMarketData, BacktestError>;
}

#[derive(Debug, Clone)]
pub struct InMemoryDataSource {
    pub name: String,
    pub loaded: LoadedMarketData,
}

impl HistoricalDataSource for InMemoryDataSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            supports_candles: matches!(self.loaded.dataset, MarketDataSet::Candles(_)),
            supports_events: matches!(self.loaded.dataset, MarketDataSet::Events(_)),
            supports_range_filtering: true,
            supports_sequence_guarantees: matches!(self.loaded.dataset, MarketDataSet::Events(_)),
        }
    }

    fn load(&self, request: &MarketDataRequest) -> Result<LoadedMarketData, BacktestError> {
        let dataset = filter_market_data(
            &self.loaded.dataset,
            request.start,
            request.end,
            request.limit,
        )?;
        let loaded = LoadedMarketData {
            dataset,
            metadata: self.loaded.metadata.clone(),
        };
        loaded.validate(request, &self.capabilities())?;
        Ok(loaded)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileDataSourceSpec {
    CandleCsv { path: PathBuf },
    EventJson { path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct FileDataSource {
    pub name: String,
    pub instrument: String,
    pub timeframe: Option<String>,
    pub spec: FileDataSourceSpec,
    pub schema: Option<SourceSchema>,
}

impl HistoricalDataSource for FileDataSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        match self.spec {
            FileDataSourceSpec::CandleCsv { .. } => DataSourceCapabilities {
                supports_candles: true,
                supports_events: false,
                supports_range_filtering: true,
                supports_sequence_guarantees: false,
            },
            FileDataSourceSpec::EventJson { .. } => DataSourceCapabilities {
                supports_candles: false,
                supports_events: true,
                supports_range_filtering: true,
                supports_sequence_guarantees: false,
            },
        }
    }

    fn load(&self, request: &MarketDataRequest) -> Result<LoadedMarketData, BacktestError> {
        request.validate()?;
        let dataset = match &self.spec {
            FileDataSourceSpec::CandleCsv { path } => {
                MarketDataSet::Candles(load_candles_from_csv(path)?)
            }
            FileDataSourceSpec::EventJson { path } => {
                MarketDataSet::Events(load_events_from_json(path)?)
            }
        };
        let loaded = LoadedMarketData {
            dataset: filter_market_data(&dataset, request.start, request.end, request.limit)?,
            metadata: DataSourceMetadata {
                provider_name: self.name.clone(),
                instrument: self.instrument.clone(),
                timeframe: self.timeframe.clone(),
                data_mode: dataset.mode_label().to_string(),
                fingerprint: None,
                source_schema: self.schema.clone(),
                notes: BTreeMap::from([(
                    "origin".to_string(),
                    match self.spec {
                        FileDataSourceSpec::CandleCsv { .. } => "file_csv".to_string(),
                        FileDataSourceSpec::EventJson { .. } => "file_json".to_string(),
                    },
                )]),
            },
        };
        loaded.validate(request, &self.capabilities())?;
        Ok(loaded)
    }
}

pub fn filter_market_data(
    dataset: &MarketDataSet,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    limit: Option<usize>,
) -> Result<MarketDataSet, BacktestError> {
    let within_range = |timestamp: DateTime<Utc>| -> bool {
        let after_start = start.map(|start| timestamp >= start).unwrap_or(true);
        let before_end = end.map(|end| timestamp <= end).unwrap_or(true);
        after_start && before_end
    };
    let filtered = match dataset {
        MarketDataSet::Candles(candles) => MarketDataSet::Candles(
            candles
                .iter()
                .filter(|candle| within_range(candle.timestamp))
                .take(limit.unwrap_or(usize::MAX))
                .cloned()
                .collect(),
        ),
        MarketDataSet::Events(events) => MarketDataSet::Events(
            events
                .iter()
                .filter(|event| within_range(event.timestamp))
                .take(limit.unwrap_or(usize::MAX))
                .cloned()
                .collect(),
        ),
    };
    if filtered.is_empty() {
        return Err(BacktestError::InvalidData(
            "provider returned no market data for requested range".to_string(),
        ));
    }
    Ok(filtered)
}

fn validate_market_data_mode(
    dataset: &MarketDataSet,
    requested_mode: &MarketDataMode,
    capabilities: &DataSourceCapabilities,
) -> Result<(), BacktestError> {
    match (requested_mode, dataset) {
        (MarketDataMode::Candles, MarketDataSet::Events(_)) => Err(BacktestError::InvalidData(
            "provider returned events for a candle request".to_string(),
        )),
        (MarketDataMode::Events, MarketDataSet::Candles(_)) => Err(BacktestError::InvalidData(
            "provider returned candles for an event request".to_string(),
        )),
        (MarketDataMode::PreferCandles, MarketDataSet::Events(_))
            if !capabilities.supports_events =>
        {
            Err(BacktestError::InvalidData(
                "provider cannot satisfy candle-preferred request with event data".to_string(),
            ))
        }
        (MarketDataMode::PreferEvents, MarketDataSet::Candles(_))
            if !capabilities.supports_candles =>
        {
            Err(BacktestError::InvalidData(
                "provider cannot satisfy event-preferred request with candle data".to_string(),
            ))
        }
        _ => Ok(()),
    }
}

fn validate_market_data_set(
    dataset: &MarketDataSet,
    capabilities: &DataSourceCapabilities,
) -> Result<(), BacktestError> {
    match dataset {
        MarketDataSet::Candles(candles) => {
            for window in candles.windows(2) {
                if window[1].timestamp <= window[0].timestamp {
                    return Err(BacktestError::InvalidData(
                        "provider candle timestamps must be strictly increasing".to_string(),
                    ));
                }
            }
            for candle in candles {
                if !candle.validate() {
                    return Err(BacktestError::InvalidData(format!(
                        "provider returned invalid candle at {}",
                        candle.timestamp
                    )));
                }
            }
        }
        MarketDataSet::Events(events) => {
            let mut last_timestamp = None;
            let mut last_sequence = None;
            for event in events {
                if !event.validate() {
                    return Err(BacktestError::InvalidData(format!(
                        "provider returned invalid event at {} sequence {}",
                        event.timestamp, event.sequence
                    )));
                }
                if let Some(timestamp) = last_timestamp {
                    if event.timestamp < timestamp {
                        return Err(BacktestError::InvalidData(
                            "provider event timestamps must be non-decreasing".to_string(),
                        ));
                    }
                    if event.timestamp == timestamp {
                        if let Some(sequence) = last_sequence {
                            if event.sequence <= sequence {
                                return Err(BacktestError::InvalidData(
                                    "provider events with equal timestamps must have strictly increasing sequence"
                                        .to_string(),
                                ));
                            }
                        }
                    } else if capabilities.supports_sequence_guarantees && event.sequence == 0 {
                        return Err(BacktestError::InvalidData(
                            "provider advertised sequence guarantees but returned zero/unspecified event sequences"
                                .to_string(),
                        ));
                    }
                }
                last_timestamp = Some(event.timestamp);
                last_sequence = Some(event.sequence);
            }
        }
    }
    Ok(())
}

fn default_prefer_candles() -> MarketDataMode {
    MarketDataMode::PreferCandles
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{Duration, TimeZone, Utc};

    use super::{
        DataSourceMetadata, FileDataSource, FileDataSourceSpec, HistoricalDataSource,
        InMemoryDataSource, LoadedMarketData, MarketDataMode, MarketDataRequest,
    };
    use crate::domain::errors::BacktestError;
    use crate::domain::types::{Candle, MarketDataSet, MarketEvent, MarketEventKind};

    #[test]
    fn in_memory_provider_filters_by_range() {
        let candles = (0..5)
            .map(|index| Candle {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                    + Duration::hours(index as i64),
                open: 100.0 + index as f64,
                high: 101.0 + index as f64,
                low: 99.0 + index as f64,
                close: 100.5 + index as f64,
                volume: 1.0,
                funding_rate: 0.0,
                spread_bps: None,
            })
            .collect();
        let provider = InMemoryDataSource {
            name: "mock".to_string(),
            loaded: LoadedMarketData {
                dataset: MarketDataSet::Candles(candles),
                metadata: DataSourceMetadata {
                    provider_name: "mock".to_string(),
                    instrument: "BTC-PERP".to_string(),
                    timeframe: Some("1h".to_string()),
                    data_mode: "candles".to_string(),
                    fingerprint: Some("abc".to_string()),
                    source_schema: None,
                    notes: Default::default(),
                },
            },
        };

        let loaded = provider
            .load(&MarketDataRequest {
                symbol: "BTC-PERP".to_string(),
                venue: None,
                timeframe: Some("1h".to_string()),
                start: Some(Utc.with_ymd_and_hms(2024, 1, 1, 1, 0, 0).unwrap()),
                end: Some(Utc.with_ymd_and_hms(2024, 1, 1, 3, 0, 0).unwrap()),
                mode: MarketDataMode::Candles,
                limit: None,
            })
            .unwrap();

        assert_eq!(loaded.dataset.candles().len(), 3);
    }

    #[test]
    fn provider_rejects_duplicate_event_sequences() {
        let provider = InMemoryDataSource {
            name: "mock".to_string(),
            loaded: LoadedMarketData {
                dataset: MarketDataSet::Events(vec![
                    MarketEvent {
                        timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                        sequence: 1,
                        kind: MarketEventKind::Trade {
                            price: 100.0,
                            quantity: 1.0,
                            aggressor: None,
                        },
                    },
                    MarketEvent {
                        timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                        sequence: 1,
                        kind: MarketEventKind::Trade {
                            price: 100.5,
                            quantity: 1.0,
                            aggressor: None,
                        },
                    },
                ]),
                metadata: DataSourceMetadata {
                    provider_name: "mock".to_string(),
                    instrument: "BTC-PERP".to_string(),
                    timeframe: Some("event".to_string()),
                    data_mode: "events".to_string(),
                    fingerprint: None,
                    source_schema: None,
                    notes: Default::default(),
                },
            },
        };

        let error = provider
            .load(&MarketDataRequest {
                symbol: "BTC-PERP".to_string(),
                venue: None,
                timeframe: Some("event".to_string()),
                start: None,
                end: None,
                mode: MarketDataMode::Events,
                limit: None,
            })
            .unwrap_err();
        assert!(matches!(error, BacktestError::InvalidData(_)));
    }

    #[test]
    fn file_data_source_reports_mode_capabilities() {
        let provider = FileDataSource {
            name: "files".to_string(),
            instrument: "BTC-PERP".to_string(),
            timeframe: Some("1h".to_string()),
            spec: FileDataSourceSpec::CandleCsv {
                path: PathBuf::from("examples/data/btc_perp_1h.csv"),
            },
            schema: None,
        };
        let capabilities = provider.capabilities();
        assert!(capabilities.supports_candles);
        assert!(!capabilities.supports_events);
    }
}
