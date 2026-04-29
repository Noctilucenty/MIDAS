use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::errors::BacktestError;
use crate::domain::types::{Candle, Signal, SignalAction};

#[derive(Debug, Deserialize)]
struct CandleRow {
    timestamp: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    #[serde(default)]
    funding_rate: Option<f64>,
    #[serde(default)]
    spread_bps: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SignalRow {
    timestamp: String,
    action: String,
    #[serde(default)]
    leverage_override: Option<f64>,
    #[serde(default)]
    limit_price: Option<f64>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct TradeCsvRow<'a> {
    entry_time: String,
    exit_time: String,
    side: &'a str,
    quantity: f64,
    leverage: f64,
    entry_price: f64,
    exit_price: f64,
    gross_pnl: f64,
    net_pnl: f64,
    fees_paid: f64,
    funding_paid: f64,
    slippage_paid: f64,
    spread_paid: f64,
    duration_seconds: i64,
    liquidated: bool,
}

fn parse_timestamp(raw: &str) -> Result<DateTime<Utc>, BacktestError> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(raw) {
        return Ok(timestamp.with_timezone(&Utc));
    }
    if let Ok(epoch_seconds) = raw.parse::<i64>() {
        return Utc.timestamp_opt(epoch_seconds, 0).single().ok_or_else(|| {
            BacktestError::TimestampParse {
                value: raw.to_string(),
                message: "invalid epoch seconds".to_string(),
            }
        });
    }
    Err(BacktestError::TimestampParse {
        value: raw.to_string(),
        message: "expected RFC3339 timestamp or epoch seconds".to_string(),
    })
}

pub fn load_candles_from_csv(path: &Path) -> Result<Vec<Candle>, BacktestError> {
    let file =
        File::open(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let mut reader = csv::Reader::from_reader(BufReader::new(file));
    reader
        .deserialize::<CandleRow>()
        .map(|row| {
            let row = row?;
            Ok(Candle {
                timestamp: parse_timestamp(&row.timestamp)?,
                open: row.open,
                high: row.high,
                low: row.low,
                close: row.close,
                volume: row.volume,
                funding_rate: row.funding_rate.unwrap_or(0.0),
                spread_bps: row.spread_bps,
            })
        })
        .collect()
}

pub fn load_signals_from_csv(path: &Path) -> Result<Vec<Signal>, BacktestError> {
    let file =
        File::open(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let mut reader = csv::Reader::from_reader(BufReader::new(file));
    reader
        .deserialize::<SignalRow>()
        .map(|row| {
            let row = row?;
            let action = match row.action.as_str() {
                "go_long" => SignalAction::GoLong,
                "go_short" => SignalAction::GoShort,
                "exit_to_flat" => SignalAction::ExitToFlat,
                "hold" => SignalAction::Hold,
                other => {
                    return Err(BacktestError::InvalidData(format!(
                        "unsupported signal action '{other}'"
                    )))
                }
            };
            Ok(Signal {
                timestamp: parse_timestamp(&row.timestamp)?,
                action,
                leverage_override: row.leverage_override,
                limit_price: row.limit_price,
                note: row.note,
            })
        })
        .collect()
}

pub fn write_trade_log_csv(
    path: &Path,
    trades: &[crate::domain::types::Trade],
) -> Result<(), BacktestError> {
    let file =
        File::create(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(file));
    for trade in trades {
        writer.serialize(TradeCsvRow {
            entry_time: trade.entry_time.to_rfc3339(),
            exit_time: trade.exit_time.to_rfc3339(),
            side: match trade.side {
                crate::domain::types::Side::Long => "long",
                crate::domain::types::Side::Short => "short",
            },
            quantity: trade.quantity,
            leverage: trade.leverage,
            entry_price: trade.entry_price,
            exit_price: trade.exit_price,
            gross_pnl: trade.gross_pnl,
            net_pnl: trade.net_pnl,
            fees_paid: trade.fees_paid,
            funding_paid: trade.funding_paid,
            slippage_paid: trade.slippage_paid,
            spread_paid: trade.spread_paid,
            duration_seconds: trade.duration_seconds,
            liquidated: trade.liquidated,
        })?;
    }
    writer
        .flush()
        .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    Ok(())
}
