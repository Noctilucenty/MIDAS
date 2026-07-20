//! Fixed-payout binary option domain model.
//!
//! Models the contract IQ Option-style demo trading uses: a fixed stake, a
//! direction (call/put), an expiry, and a payout ratio quoted at entry.
//! Settlement is all-or-nothing: win pays `stake * payout`, loss forfeits the
//! stake, and a tie (settlement price exactly equal to entry price) resolves
//! according to the broker's [`TieBehavior`].
//!
//! This module is pure domain logic: no engine wiring, no clock, no I/O.
//! Every function is deterministic in its inputs so journaled trades can be
//! re-settled byte-for-byte during validation.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::errors::BacktestError;

/// Action emitted by a prediction model for a binary option opportunity.
///
/// `NoTrade` is a first-class outcome so coverage (share of opportunities
/// actually traded) can be measured honestly.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BinaryAction {
    BinaryCall,
    BinaryPut,
    NoTrade,
}

impl BinaryAction {
    /// Whether this action opens a position.
    pub fn is_trade(self) -> bool {
        !matches!(self, BinaryAction::NoTrade)
    }
}

/// How the broker resolves a settlement price exactly equal to entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TieBehavior {
    /// Stake returned, zero P&L (IQ Option's behavior).
    RefundStake,
    /// Tie counts as a loss.
    LoseStake,
    /// Tie counts as a win.
    WinPayout,
}

/// A timestamped binary-option signal produced by an external model.
///
/// Carries full provenance (`predicted_prob_up`, `model_version`,
/// `feature_hash`) so any backtest result can be traced to the exact model
/// and feature set that generated it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinarySignal {
    pub timestamp: DateTime<Utc>,
    pub action: BinaryAction,
    /// Fixed stake in account currency. Must be > 0 for trade actions.
    pub stake: f64,
    /// Contract length in seconds (e.g. 60, 300).
    pub expiry_seconds: u32,
    /// Payout ratio quoted at entry (0.85 = 85% profit on a win).
    pub payout: f64,
    /// Calibrated model probability that price finishes above entry.
    #[serde(default)]
    pub predicted_prob_up: Option<f64>,
    #[serde(default)]
    pub model_version: Option<String>,
    #[serde(default)]
    pub feature_hash: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

impl BinarySignal {
    /// Validate invariants for a signal before it may enter a backtest.
    pub fn validate(&self) -> Result<(), BacktestError> {
        if self.action.is_trade() {
            if !(self.stake > 0.0) {
                return Err(BacktestError::InvalidData(format!(
                    "binary signal at {} has non-positive stake {}",
                    self.timestamp, self.stake
                )));
            }
            if self.expiry_seconds == 0 {
                return Err(BacktestError::InvalidData(format!(
                    "binary signal at {} has zero expiry",
                    self.timestamp
                )));
            }
            if !(self.payout > 0.0) || self.payout > 1.0 {
                return Err(BacktestError::InvalidData(format!(
                    "binary signal at {} has payout {} outside (0, 1]",
                    self.timestamp, self.payout
                )));
            }
        }
        if let Some(p) = self.predicted_prob_up {
            if !(0.0..=1.0).contains(&p) {
                return Err(BacktestError::InvalidData(format!(
                    "binary signal at {} has probability {} outside [0, 1]",
                    self.timestamp, p
                )));
            }
        }
        Ok(())
    }
}

/// An opened binary position awaiting settlement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryPosition {
    pub signal: BinarySignal,
    pub entry_time: DateTime<Utc>,
    pub entry_price: f64,
}

impl BinaryPosition {
    pub fn expiry_time(&self) -> DateTime<Utc> {
        self.entry_time + Duration::seconds(i64::from(self.signal.expiry_seconds))
    }

    /// Whether this position is still open at `t` (settles exactly at expiry).
    pub fn is_open_at(&self, t: DateTime<Utc>) -> bool {
        t >= self.entry_time && t < self.expiry_time()
    }
}

/// Result category of a settled binary option.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOutcome {
    Win,
    Loss,
    Tie,
}

/// A settled binary trade: the position plus its resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinarySettlement {
    pub position: BinaryPosition,
    pub settlement_price: f64,
    pub outcome: BinaryOutcome,
    /// Realized P&L: win `+stake * payout`, loss `-stake`, tie per behavior.
    pub pnl: f64,
}

/// Win rate required for zero expected value at a payout ratio.
///
/// EV per unit stake = p * payout - (1 - p); zero at p = 1 / (1 + payout).
/// At 0.85 payout this is ~54.05%, which is why raw win rate alone is not an
/// edge measure.
pub fn break_even_win_rate(payout: f64) -> f64 {
    1.0 / (1.0 + payout)
}

/// Expected value per unit stake for a directional probability `p_correct`.
pub fn expected_value_per_stake(p_correct: f64, payout: f64) -> f64 {
    p_correct * payout - (1.0 - p_correct)
}

/// Settle an open position against the price at expiry.
pub fn settle(
    position: &BinaryPosition,
    settlement_price: f64,
    tie_behavior: TieBehavior,
) -> Result<BinarySettlement, BacktestError> {
    let direction_won = match position.signal.action {
        BinaryAction::BinaryCall => {
            if settlement_price > position.entry_price {
                Some(true)
            } else if settlement_price < position.entry_price {
                Some(false)
            } else {
                None
            }
        }
        BinaryAction::BinaryPut => {
            if settlement_price < position.entry_price {
                Some(true)
            } else if settlement_price > position.entry_price {
                Some(false)
            } else {
                None
            }
        }
        BinaryAction::NoTrade => {
            return Err(BacktestError::InvalidData(
                "cannot settle a no_trade signal".to_string(),
            ))
        }
    };

    let stake = position.signal.stake;
    let payout = position.signal.payout;
    let (outcome, pnl) = match direction_won {
        Some(true) => (BinaryOutcome::Win, stake * payout),
        Some(false) => (BinaryOutcome::Loss, -stake),
        None => match tie_behavior {
            TieBehavior::RefundStake => (BinaryOutcome::Tie, 0.0),
            TieBehavior::LoseStake => (BinaryOutcome::Tie, -stake),
            TieBehavior::WinPayout => (BinaryOutcome::Tie, stake * payout),
        },
    };

    Ok(BinarySettlement {
        position: position.clone(),
        settlement_price,
        outcome,
        pnl,
    })
}

/// Greatest number of positions simultaneously open at any instant.
///
/// Positions are half-open intervals `[entry, expiry)`; a position entered at
/// the exact instant another expires does not overlap it.
pub fn max_concurrent_positions(positions: &[BinaryPosition]) -> usize {
    let mut events: Vec<(DateTime<Utc>, i32)> = Vec::with_capacity(positions.len() * 2);
    for p in positions {
        events.push((p.entry_time, 1));
        events.push((p.expiry_time(), -1));
    }
    // Sort by time; at equal timestamps process expiries (-1) before entries.
    events.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut current = 0i32;
    let mut peak = 0i32;
    for (_, delta) in events {
        current += delta;
        peak = peak.max(current);
    }
    peak.max(0) as usize
}

/// Reject position sets that exceed the allowed concurrent-position limit.
pub fn enforce_position_limit(
    positions: &[BinaryPosition],
    max_allowed: usize,
) -> Result<(), BacktestError> {
    let peak = max_concurrent_positions(positions);
    if peak > max_allowed {
        return Err(BacktestError::InvariantViolation {
            name: "binary_position_limit".to_string(),
            detail: format!("{peak} concurrent positions exceeds limit {max_allowed}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn signal(action: BinaryAction) -> BinarySignal {
        BinarySignal {
            timestamp: t(0),
            action,
            stake: 1.0,
            expiry_seconds: 60,
            payout: 0.85,
            predicted_prob_up: Some(0.6),
            model_version: Some("logreg-1.0.0".to_string()),
            feature_hash: Some("abc123".to_string()),
            note: None,
        }
    }

    fn position(action: BinaryAction, entry_price: f64) -> BinaryPosition {
        BinaryPosition {
            signal: signal(action),
            entry_time: t(0),
            entry_price,
        }
    }

    #[test]
    fn call_win_pays_stake_times_payout() {
        let s = settle(&position(BinaryAction::BinaryCall, 1.10), 1.11, TieBehavior::RefundStake)
            .unwrap();
        assert_eq!(s.outcome, BinaryOutcome::Win);
        assert!((s.pnl - 0.85).abs() < 1e-12);
    }

    #[test]
    fn call_loss_forfeits_stake() {
        let s = settle(&position(BinaryAction::BinaryCall, 1.10), 1.09, TieBehavior::RefundStake)
            .unwrap();
        assert_eq!(s.outcome, BinaryOutcome::Loss);
        assert!((s.pnl + 1.0).abs() < 1e-12);
    }

    #[test]
    fn put_wins_when_price_falls() {
        let s = settle(&position(BinaryAction::BinaryPut, 1.10), 1.09, TieBehavior::RefundStake)
            .unwrap();
        assert_eq!(s.outcome, BinaryOutcome::Win);
        assert!((s.pnl - 0.85).abs() < 1e-12);
    }

    #[test]
    fn tie_behaviors() {
        let p = position(BinaryAction::BinaryCall, 1.10);
        let refund = settle(&p, 1.10, TieBehavior::RefundStake).unwrap();
        assert_eq!(refund.outcome, BinaryOutcome::Tie);
        assert_eq!(refund.pnl, 0.0);
        let lose = settle(&p, 1.10, TieBehavior::LoseStake).unwrap();
        assert!((lose.pnl + 1.0).abs() < 1e-12);
        let win = settle(&p, 1.10, TieBehavior::WinPayout).unwrap();
        assert!((win.pnl - 0.85).abs() < 1e-12);
    }

    #[test]
    fn no_trade_cannot_settle() {
        assert!(settle(&position(BinaryAction::NoTrade, 1.10), 1.11, TieBehavior::RefundStake)
            .is_err());
    }

    #[test]
    fn break_even_matches_known_values() {
        assert!((break_even_win_rate(0.85) - 0.5405).abs() < 0.0001);
        assert!((break_even_win_rate(1.0) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn expected_value_is_zero_at_break_even() {
        let payout = 0.85;
        let ev = expected_value_per_stake(break_even_win_rate(payout), payout);
        assert!(ev.abs() < 1e-12);
        assert!(expected_value_per_stake(0.60, payout) > 0.0);
        assert!(expected_value_per_stake(0.50, payout) < 0.0);
    }

    #[test]
    fn validate_rejects_bad_signals() {
        let mut s = signal(BinaryAction::BinaryCall);
        s.stake = 0.0;
        assert!(s.validate().is_err());

        let mut s = signal(BinaryAction::BinaryCall);
        s.payout = 1.5;
        assert!(s.validate().is_err());

        let mut s = signal(BinaryAction::BinaryCall);
        s.expiry_seconds = 0;
        assert!(s.validate().is_err());

        let mut s = signal(BinaryAction::BinaryCall);
        s.predicted_prob_up = Some(1.2);
        assert!(s.validate().is_err());

        // NoTrade carries no stake requirements.
        let mut s = signal(BinaryAction::NoTrade);
        s.stake = 0.0;
        assert!(s.validate().is_ok());
    }

    #[test]
    fn position_open_interval_is_half_open() {
        let p = position(BinaryAction::BinaryCall, 1.10);
        assert!(p.is_open_at(t(0)));
        assert!(p.is_open_at(t(59)));
        assert!(!p.is_open_at(t(60)));
    }

    #[test]
    fn concurrent_position_counting() {
        let mut a = position(BinaryAction::BinaryCall, 1.10); // [0, 60)
        a.entry_time = t(0);
        let mut b = position(BinaryAction::BinaryPut, 1.10); // [30, 90)
        b.entry_time = t(30);
        let mut c = position(BinaryAction::BinaryCall, 1.10); // [60, 120) - starts as `a` expires
        c.entry_time = t(60);

        assert_eq!(max_concurrent_positions(&[a.clone()]), 1);
        assert_eq!(max_concurrent_positions(&[a.clone(), b.clone()]), 2);
        assert_eq!(max_concurrent_positions(&[a.clone(), c.clone()]), 1);
        assert_eq!(max_concurrent_positions(&[a.clone(), b.clone(), c.clone()]), 2);

        assert!(enforce_position_limit(&[a.clone(), b.clone()], 2).is_ok());
        assert!(enforce_position_limit(&[a, b, c], 1).is_err());
    }

    #[test]
    fn serde_uses_snake_case_tags() {
        let json = serde_json::to_string(&BinaryAction::BinaryCall).unwrap();
        assert_eq!(json, "\"binary_call\"");
        let json = serde_json::to_string(&TieBehavior::RefundStake).unwrap();
        assert_eq!(json, "\"refund_stake\"");
    }
}
