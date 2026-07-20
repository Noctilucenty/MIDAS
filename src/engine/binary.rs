//! Deterministic replay engine for fixed-payout binary option signals.
//!
//! Takes a candle series and a timestamped [`BinarySignal`] stream (produced
//! by an external, already-frozen model) and settles every trade against the
//! candles. No fitting happens here: this engine is pure accounting, which is
//! what makes its results usable as validation evidence.
//!
//! Conventions (all deterministic and documented so Python-side journals can
//! be replayed byte-for-byte):
//! - A signal with timestamp T enters at the first candle whose timestamp is
//!   >= T, at that candle's OPEN (next-bar-open; the signal was computed from
//!   bars that closed at or before T).
//! - Settlement price is the CLOSE of the bar covering the expiry instant
//!   `entry_time + expiry_seconds` (the bar with the greatest timestamp
//!   strictly below expiry whose span reaches it).
//! - Signals that cannot enter or settle (gaps, end of data) are counted in
//!   `skipped_missing_data`, never silently dropped.
//! - At most `max_concurrent` positions may be open; excess signals are
//!   counted in `skipped_overlap`.
//! - `no_trade` signals count toward opportunities only.

use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::domain::binary::{
    settle, BinaryPosition, BinarySettlement, BinarySignal, TieBehavior,
};
use crate::domain::errors::BacktestError;
use crate::domain::types::Candle;
use crate::metrics::binary::{compute_binary_metrics, BinaryMetricsReport};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryBacktestConfig {
    pub tie_behavior: TieBehavior,
    /// Maximum simultaneously open positions (risk engine constraint).
    pub max_concurrent: usize,
}

impl Default for BinaryBacktestConfig {
    fn default() -> Self {
        Self {
            tie_behavior: TieBehavior::RefundStake,
            max_concurrent: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryBacktestReport {
    pub opportunities: usize,
    pub executed: usize,
    pub skipped_missing_data: usize,
    pub skipped_overlap: usize,
    pub metrics: BinaryMetricsReport,
    pub settlements: Vec<BinarySettlement>,
}

/// Replay `signals` over `candles` and settle every executed trade.
pub fn run_binary_backtest(
    candles: &[Candle],
    signals: &[BinarySignal],
    config: &BinaryBacktestConfig,
) -> Result<BinaryBacktestReport, BacktestError> {
    if candles.len() < 2 {
        return Err(BacktestError::InvalidData(
            "binary backtest needs at least two candles".to_string(),
        ));
    }
    let mut sorted_candles: Vec<&Candle> = candles.iter().collect();
    sorted_candles.sort_by_key(|c| c.timestamp);
    let interval = sorted_candles[1].timestamp - sorted_candles[0].timestamp;
    if interval <= Duration::zero() {
        return Err(BacktestError::InvalidData(
            "candle timestamps must be strictly increasing".to_string(),
        ));
    }

    let mut sorted_signals: Vec<&BinarySignal> = signals.iter().collect();
    sorted_signals.sort_by_key(|s| s.timestamp);

    let mut settlements: Vec<BinarySettlement> = Vec::new();
    let mut open_until: Vec<chrono::DateTime<chrono::Utc>> = Vec::new();
    let mut skipped_missing_data = 0usize;
    let mut skipped_overlap = 0usize;

    for signal in &sorted_signals {
        signal.validate()?;
        if !signal.action.is_trade() {
            continue;
        }

        // Entry: first candle at or after the signal timestamp.
        let Some(entry_candle) = sorted_candles
            .iter()
            .find(|c| c.timestamp >= signal.timestamp)
        else {
            skipped_missing_data += 1;
            continue;
        };
        let entry_time = entry_candle.timestamp;

        // Concurrency limit against positions still open at entry.
        open_until.retain(|expiry| *expiry > entry_time);
        if open_until.len() >= config.max_concurrent {
            skipped_overlap += 1;
            continue;
        }

        let position = BinaryPosition {
            signal: (*signal).clone(),
            entry_time,
            entry_price: entry_candle.open,
        };
        let expiry_time = position.expiry_time();

        // Settlement bar: greatest timestamp strictly below expiry whose span
        // reaches the expiry instant.
        let settlement_candle = sorted_candles
            .iter()
            .rev()
            .find(|c| c.timestamp < expiry_time && c.timestamp + interval >= expiry_time);
        let Some(settlement_candle) = settlement_candle else {
            skipped_missing_data += 1;
            continue;
        };

        settlements.push(settle(
            &position,
            settlement_candle.close,
            config.tie_behavior,
        )?);
        open_until.push(expiry_time);
    }

    let opportunities = sorted_signals.len();
    let metrics = compute_binary_metrics(&settlements, opportunities);
    Ok(BinaryBacktestReport {
        opportunities,
        executed: settlements.len(),
        skipped_missing_data,
        skipped_overlap,
        metrics,
        settlements,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::binary::BinaryAction;
    use chrono::{DateTime, TimeZone, Utc};

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn candle(secs: i64, open: f64, close: f64) -> Candle {
        Candle {
            timestamp: t(secs),
            open,
            high: open.max(close) + 0.001,
            low: open.min(close) - 0.001,
            close,
            volume: 1.0,
            funding_rate: 0.0,
            spread_bps: None,
        }
    }

    fn rising_candles(n: i64) -> Vec<Candle> {
        (0..n)
            .map(|i| candle(i * 60, 1.0 + i as f64 * 0.01, 1.0 + (i + 1) as f64 * 0.01))
            .collect()
    }

    fn signal(secs: i64, action: BinaryAction) -> BinarySignal {
        BinarySignal {
            timestamp: t(secs),
            action,
            stake: 1.0,
            expiry_seconds: 300,
            payout: 0.85,
            predicted_prob_up: Some(0.6),
            model_version: Some("test".to_string()),
            feature_hash: None,
            note: None,
        }
    }

    #[test]
    fn call_on_rising_series_wins() {
        let report = run_binary_backtest(
            &rising_candles(20),
            &[signal(60, BinaryAction::BinaryCall)],
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert_eq!(report.executed, 1);
        let s = &report.settlements[0];
        // Enters at candle t=60 open (1.01); settles at close of bar covering
        // t=360, i.e. bar t=300..360 with close 1.06.
        assert!((s.position.entry_price - 1.01).abs() < 1e-9);
        assert!((s.settlement_price - 1.06).abs() < 1e-9);
        assert!((s.pnl - 0.85).abs() < 1e-9);
    }

    #[test]
    fn put_on_rising_series_loses() {
        let report = run_binary_backtest(
            &rising_candles(20),
            &[signal(60, BinaryAction::BinaryPut)],
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert!((report.settlements[0].pnl + 1.0).abs() < 1e-9);
    }

    #[test]
    fn no_trade_counts_as_opportunity_only() {
        let report = run_binary_backtest(
            &rising_candles(20),
            &[signal(60, BinaryAction::NoTrade), signal(120, BinaryAction::BinaryCall)],
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert_eq!(report.opportunities, 2);
        assert_eq!(report.executed, 1);
        assert!((report.metrics.coverage.unwrap() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn concurrency_limit_skips_overlapping_signals() {
        let signals = vec![
            signal(60, BinaryAction::BinaryCall),
            signal(120, BinaryAction::BinaryCall), // overlaps the first (expiry 360)
            signal(420, BinaryAction::BinaryCall), // first has expired
        ];
        let report = run_binary_backtest(
            &rising_candles(20),
            &signals,
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert_eq!(report.executed, 2);
        assert_eq!(report.skipped_overlap, 1);

        let relaxed = run_binary_backtest(
            &rising_candles(20),
            &signals,
            &BinaryBacktestConfig {
                max_concurrent: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(relaxed.executed, 3);
    }

    #[test]
    fn expiry_beyond_data_is_skipped_not_dropped_silently() {
        let report = run_binary_backtest(
            &rising_candles(6), // data ends at t=360
            &[signal(300, BinaryAction::BinaryCall)], // entry 300, expiry 600
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert_eq!(report.executed, 0);
        assert_eq!(report.skipped_missing_data, 1);
    }

    #[test]
    fn signal_after_all_candles_is_missing_data() {
        let report = run_binary_backtest(
            &rising_candles(5),
            &[signal(10_000, BinaryAction::BinaryCall)],
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert_eq!(report.skipped_missing_data, 1);
    }

    #[test]
    fn invalid_signal_is_an_error_not_a_skip() {
        let mut bad = signal(60, BinaryAction::BinaryCall);
        bad.payout = 2.0;
        assert!(run_binary_backtest(
            &rising_candles(20),
            &[bad],
            &BinaryBacktestConfig::default()
        )
        .is_err());
    }
}
