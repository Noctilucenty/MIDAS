use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::adapters::csv::{load_candles_from_csv, load_signals_from_csv};
use crate::adapters::json::load_events_from_json;
use crate::adapters::provider::{HistoricalDataSource, MarketDataRequest};
use crate::domain::config::{
    BacktestConfig, BacktestRequest, ExecutionConfig, RunContext, StrategyDefinition,
    ValidationConfig, ValidationRequest,
};
use crate::domain::errors::BacktestError;
use crate::domain::types::{MarketDataSet, StrategyInput};
use crate::engine::backtester::BacktestEngine;
use crate::validation::run_validation;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarketDataFileSpec {
    OhlcvCsv { path: PathBuf },
    EventJson { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StrategyFileSpec {
    MovingAverageCross {
        fast_window: usize,
        slow_window: usize,
    },
    SignalCsv {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileBacktestRequest {
    pub context: RunContext,
    pub market_data: MarketDataFileSpec,
    pub strategy: StrategyFileSpec,
    pub backtest_config: BacktestConfig,
    pub execution_config: ExecutionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileValidationRequest {
    pub backtest: FileBacktestRequest,
    pub validation_config: ValidationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderBacktestRequest {
    pub context: RunContext,
    pub data_request: MarketDataRequest,
    pub strategy_input: StrategyInput,
    pub backtest_config: BacktestConfig,
    pub execution_config: ExecutionConfig,
}

pub struct BacktestService;

impl BacktestService {
    pub fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, BacktestError> {
        let file = File::open(path)
            .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
        let reader = BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    }

    pub fn run_backtest_from_file_request(
        file_request: FileBacktestRequest,
    ) -> Result<crate::domain::types::BacktestReport, BacktestError> {
        let request = Self::materialize_backtest_request(file_request)?;
        BacktestEngine::run(request, 0)
    }

    pub fn run_validation_from_file_request(
        file_request: FileValidationRequest,
    ) -> Result<crate::domain::types::ValidationReport, BacktestError> {
        let request = ValidationRequest {
            backtest_request: Self::materialize_backtest_request(file_request.backtest)?,
            validation_config: file_request.validation_config,
        };
        run_validation(request)
    }

    pub fn run_backtest_with_data_source(
        provider: &dyn HistoricalDataSource,
        request: ProviderBacktestRequest,
        deterministic_seed: u64,
    ) -> Result<crate::domain::types::BacktestReport, BacktestError> {
        request.data_request.validate()?;
        let loaded = provider.load(&request.data_request)?;
        loaded.validate(&request.data_request, &provider.capabilities())?;
        let mut context = request.context;
        if context.venue.is_none() {
            context.venue = Some(loaded.metadata.provider_name);
        }
        let backtest_request = BacktestRequest {
            context,
            market_data: loaded.dataset,
            strategy_input: request.strategy_input,
            backtest_config: request.backtest_config,
            execution_config: request.execution_config,
        };
        BacktestEngine::run(backtest_request, deterministic_seed)
    }

    fn materialize_backtest_request(
        file_request: FileBacktestRequest,
    ) -> Result<BacktestRequest, BacktestError> {
        let market_data = match file_request.market_data {
            MarketDataFileSpec::OhlcvCsv { path } => {
                MarketDataSet::Candles(load_candles_from_csv(&path)?)
            }
            MarketDataFileSpec::EventJson { path } => {
                MarketDataSet::Events(load_events_from_json(&path)?)
            }
        };
        let strategy_input = match file_request.strategy {
            StrategyFileSpec::MovingAverageCross {
                fast_window,
                slow_window,
            } => StrategyInput::Definition(StrategyDefinition::MovingAverageCross {
                fast_window,
                slow_window,
            }),
            StrategyFileSpec::SignalCsv { path } => {
                StrategyInput::SignalStream(load_signals_from_csv(&path)?)
            }
        };
        Ok(BacktestRequest {
            context: file_request.context,
            market_data,
            strategy_input,
            backtest_config: file_request.backtest_config,
            execution_config: file_request.execution_config,
        })
    }
}
