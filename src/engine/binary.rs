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
    /// Every decision the model made, including no_trade.
    pub opportunities: usize,
    /// Decisions where the policy chose no_trade (model abstention).
    pub no_trade_decisions: usize,
    /// Decisions where the policy wanted a trade (call or put).
    pub trade_intents: usize,
    /// trade_intents / opportunities: how often the MODEL wanted to trade.
    pub policy_coverage: Option<f64>,
    /// Intents rejected by the concurrency limit (risk engine, NOT abstention).
    pub skipped_overlap: usize,
    /// Intents rejected because entry was stale or data was missing at
    /// entry/settlement (gaps, end of data).
    pub skipped_missing_data: usize,
    /// Intents rejected because no candle within one interval of the signal
    /// timestamp existed (entering later would be a different trade).
    pub skipped_stale_entry: usize,
    /// Trades actually entered and settled.
    pub executed: usize,
    /// executed / opportunities: how often a trade actually happened.
    pub execution_coverage: Option<f64>,
    pub metrics: BinaryMetricsReport,
    pub settlements: Vec<BinarySettlement>,
}

/// Replay `signals` over `candles` and settle every executed trade.
pub fn run_binary_backtest(
    candles: &[Candle],
    signals: &[BinarySignal],
    config: &BinaryBacktestConfig,
) -> Result<BinaryBacktestReport, BacktestError> {
    if config.max_concurrent < 1 {
        return Err(BacktestError::InvalidConfig(
            "max_concurrent must be >= 1".to_string(),
        ));
    }
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
    // Validate the whole series: every step must be the base interval
    // (contiguous) or a whole multiple of it (an explicit gap). Irregular
    // spacing would silently corrupt entry/settlement alignment.
    for pair in sorted_candles.windows(2) {
        let step = pair[1].timestamp - pair[0].timestamp;
        let step_ms = step.num_milliseconds();
        let interval_ms = interval.num_milliseconds();
        if step_ms <= 0 || step_ms % interval_ms != 0 {
            return Err(BacktestError::InvalidData(format!(
                "irregular candle spacing at {}: step {}s is not a positive multiple of {}s",
                pair[1].timestamp,
                step_ms as f64 / 1000.0,
                interval_ms as f64 / 1000.0
            )));
        }
    }

    let mut sorted_signals: Vec<&BinarySignal> = signals.iter().collect();
    sorted_signals.sort_by_key(|s| s.timestamp);

    let mut settlements: Vec<BinarySettlement> = Vec::new();
    let mut open_until: Vec<chrono::DateTime<chrono::Utc>> = Vec::new();
    let mut no_trade_decisions = 0usize;
    let mut skipped_missing_data = 0usize;
    let mut skipped_overlap = 0usize;
    let mut skipped_stale_entry = 0usize;

    for signal in &sorted_signals {
        signal.validate()?;
        if !signal.action.is_trade() {
            no_trade_decisions += 1;
            continue;
        }
        if i64::from(signal.expiry_seconds) * 1000 % interval.num_milliseconds() != 0 {
            return Err(BacktestError::InvalidData(format!(
                "signal at {} has expiry {}s that is not a multiple of the {}s candle interval",
                signal.timestamp,
                signal.expiry_seconds,
                interval.num_milliseconds() / 1000
            )));
        }

        // Entry: first candle at or after the signal timestamp...
        let Some(entry_candle) = sorted_candles
            .iter()
            .find(|c| c.timestamp >= signal.timestamp)
        else {
            skipped_missing_data += 1;
            continue;
        };
        // ...but only if it is the IMMEDIATE next bar. Entering after a gap
        // would execute a different trade than the model intended.
        if entry_candle.timestamp - signal.timestamp >= interval {
            skipped_stale_entry += 1;
            continue;
        }
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

        // Settlement requires the bar ending EXACTLY at expiry. A bar merely
        // near the expiry is a different settlement price.
        let settlement_candle = sorted_candles
            .iter()
            .rev()
            .find(|c| c.timestamp + interval == expiry_time);
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
    let trade_intents = opportunities - no_trade_decisions;
    let executed = settlements.len();
    let metrics = compute_binary_metrics(&settlements, opportunities);
    Ok(BinaryBacktestReport {
        opportunities,
        no_trade_decisions,
        trade_intents,
        policy_coverage: (opportunities > 0)
            .then(|| trade_intents as f64 / opportunities as f64),
        skipped_overlap,
        skipped_missing_data,
        skipped_stale_entry,
        executed,
        execution_coverage: (opportunities > 0)
            .then(|| executed as f64 / opportunities as f64),
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
        assert_eq!(report.no_trade_decisions, 1);
        assert_eq!(report.trade_intents, 1);
        assert_eq!(report.executed, 1);
        assert!((report.policy_coverage.unwrap() - 0.5).abs() < 1e-12);
        assert!((report.execution_coverage.unwrap() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn abstention_and_concurrency_rejection_are_reported_separately() {
        // 1 abstention + 3 intents, of which 1 is rejected by concurrency.
        let signals = vec![
            signal(60, BinaryAction::NoTrade),
            signal(120, BinaryAction::BinaryCall),
            signal(180, BinaryAction::BinaryCall), // overlaps
            signal(600, BinaryAction::BinaryCall),
        ];
        let report = run_binary_backtest(
            &rising_candles(20),
            &signals,
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert_eq!(report.opportunities, 4);
        assert_eq!(report.no_trade_decisions, 1);
        assert_eq!(report.trade_intents, 3);
        assert_eq!(report.skipped_overlap, 1);
        assert_eq!(report.executed, 2);
        assert!((report.policy_coverage.unwrap() - 0.75).abs() < 1e-12);
        assert!((report.execution_coverage.unwrap() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn entry_delayed_across_gap_is_rejected_as_stale() {
        // Contiguous bars to t=240, then a gap, resuming at t=600.
        let mut candles = rising_candles(5); // t = 0..240
        candles.push(candle(600, 1.05, 1.06));
        candles.push(candle(660, 1.06, 1.07));
        let report = run_binary_backtest(
            &candles,
            &[signal(300, BinaryAction::BinaryCall)], // next candle is 600: stale
            &BinaryBacktestConfig::default(),
        )
        .unwrap();
        assert_eq!(report.executed, 0);
        assert_eq!(report.skipped_stale_entry, 1);
    }

    #[test]
    fn irregular_candle_spacing_is_rejected() {
        let candles = vec![
            candle(0, 1.0, 1.01),
            candle(60, 1.01, 1.02),
            candle(90, 1.02, 1.03), // 30s step: not a multiple of 60
        ];
        assert!(run_binary_backtest(
            &candles,
            &[signal(0, BinaryAction::BinaryCall)],
            &BinaryBacktestConfig::default()
        )
        .is_err());
    }

    #[test]
    fn expiry_not_multiple_of_interval_is_rejected() {
        let mut odd = signal(60, BinaryAction::BinaryCall);
        odd.expiry_seconds = 90;
        assert!(run_binary_backtest(
            &rising_candles(20),
            &[odd],
            &BinaryBacktestConfig::default()
        )
        .is_err());
    }

    #[test]
    fn zero_max_concurrent_is_invalid() {
        assert!(run_binary_backtest(
            &rising_candles(20),
            &[signal(60, BinaryAction::BinaryCall)],
            &BinaryBacktestConfig {
                max_concurrent: 0,
                ..Default::default()
            }
        )
        .is_err());
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
