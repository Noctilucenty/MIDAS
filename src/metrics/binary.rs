//! Metrics for settled fixed-payout binary option trades.
//!
//! Binary options need payout-aware measures: a 60% win rate is *losing* money
//! at a 0.60 payout, so win rate is always reported next to the
//! stake-weighted break-even rate and the realized edge over it. `NoTrade`
//! decisions enter via `opportunities` so coverage is measured honestly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::binary::{break_even_win_rate, BinaryOutcome, BinarySettlement};

/// Aggregate report over a set of settled binary trades.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryMetricsReport {
    /// Decisions the model was offered (trades + no-trades). 0 = unknown.
    pub opportunities: usize,
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub ties: usize,
    /// Wins / (wins + losses); ties excluded. None if no decisive trades.
    pub win_rate: Option<f64>,
    /// Stake-weighted average payout across trades.
    pub avg_payout: Option<f64>,
    /// Break-even win rate at the stake-weighted average payout.
    pub break_even_win_rate: Option<f64>,
    /// win_rate - break_even_win_rate. Positive = beating the payout math.
    pub edge_over_break_even: Option<f64>,
    pub total_staked: f64,
    pub net_pnl: f64,
    /// Mean P&L per trade in account currency.
    pub ev_per_trade: Option<f64>,
    /// Peak-to-trough drop of the cumulative P&L curve (>= 0).
    pub max_drawdown: f64,
    /// Trades taken / opportunities offered. None if opportunities unknown.
    pub coverage: Option<f64>,
    pub longest_losing_streak: usize,
    /// Mean squared error of predicted up-probability vs realized direction,
    /// over decisive trades that carried a prediction. Lower is better;
    /// 0.25 is the score of always predicting 0.5.
    pub brier_score: Option<f64>,
    /// Share of decisive trades whose price finished above entry - the class
    /// prior a constant predictor would use. Baseline Brier = p * (1 - p).
    pub label_up_rate: Option<f64>,
}

/// Compute the aggregate report. `opportunities` counts every decision the
/// model made including `NoTrade` (pass 0 when unknown).
pub fn compute_binary_metrics(
    settlements: &[BinarySettlement],
    opportunities: usize,
) -> BinaryMetricsReport {
    let trades = settlements.len();
    let wins = count(settlements, BinaryOutcome::Win);
    let losses = count(settlements, BinaryOutcome::Loss);
    let ties = count(settlements, BinaryOutcome::Tie);
    let decisive = wins + losses;

    let win_rate = (decisive > 0).then(|| wins as f64 / decisive as f64);

    let total_staked: f64 = settlements.iter().map(|s| s.position.signal.stake).sum();
    let avg_payout = (total_staked > 0.0).then(|| {
        settlements
            .iter()
            .map(|s| s.position.signal.payout * s.position.signal.stake)
            .sum::<f64>()
            / total_staked
    });
    let break_even = avg_payout.map(break_even_win_rate);
    let edge_over_break_even = match (win_rate, break_even) {
        (Some(w), Some(b)) => Some(w - b),
        _ => None,
    };

    let net_pnl: f64 = settlements.iter().map(|s| s.pnl).sum();
    let ev_per_trade = (trades > 0).then(|| net_pnl / trades as f64);

    let mut equity = 0.0f64;
    let mut peak = 0.0f64;
    let mut max_drawdown = 0.0f64;
    for s in settlements {
        equity += s.pnl;
        peak = peak.max(equity);
        max_drawdown = max_drawdown.max(peak - equity);
    }

    let coverage = (opportunities > 0).then(|| trades as f64 / opportunities as f64);

    let mut streak = 0usize;
    let mut longest_losing_streak = 0usize;
    for s in settlements {
        match s.outcome {
            BinaryOutcome::Loss => {
                streak += 1;
                longest_losing_streak = longest_losing_streak.max(streak);
            }
            // A tie is not a loss; it breaks the streak by definition here.
            BinaryOutcome::Win | BinaryOutcome::Tie => streak = 0,
        }
    }

    let brier_score = brier(settlements);
    let label_up_rate = up_rate(settlements);

    BinaryMetricsReport {
        opportunities,
        trades,
        wins,
        losses,
        ties,
        win_rate,
        avg_payout,
        break_even_win_rate: break_even,
        edge_over_break_even,
        total_staked,
        net_pnl,
        ev_per_trade,
        max_drawdown,
        coverage,
        longest_losing_streak,
        brier_score,
        label_up_rate,
    }
}

/// Whether a decisive settlement's price finished above entry.
fn went_up(s: &BinarySettlement) -> Option<bool> {
    use crate::domain::binary::BinaryAction;
    match (s.position.signal.action, s.outcome) {
        (BinaryAction::BinaryCall, BinaryOutcome::Win) => Some(true),
        (BinaryAction::BinaryCall, BinaryOutcome::Loss) => Some(false),
        (BinaryAction::BinaryPut, BinaryOutcome::Win) => Some(false),
        (BinaryAction::BinaryPut, BinaryOutcome::Loss) => Some(true),
        _ => None,
    }
}

fn up_rate(settlements: &[BinarySettlement]) -> Option<f64> {
    let labels: Vec<bool> = settlements.iter().filter_map(went_up).collect();
    (!labels.is_empty())
        .then(|| labels.iter().filter(|up| **up).count() as f64 / labels.len() as f64)
}

/// Group settlements by a key and compute per-group reports (e.g. by payout
/// bucket, expiry, session or volatility regime).
pub fn compute_binary_metrics_by<F>(
    settlements: &[BinarySettlement],
    key_fn: F,
) -> BTreeMap<String, BinaryMetricsReport>
where
    F: Fn(&BinarySettlement) -> String,
{
    let mut groups: BTreeMap<String, Vec<BinarySettlement>> = BTreeMap::new();
    for s in settlements {
        groups.entry(key_fn(s)).or_default().push(s.clone());
    }
    groups
        .into_iter()
        .map(|(key, group)| (key, compute_binary_metrics(&group, 0)))
        .collect()
}

fn count(settlements: &[BinarySettlement], outcome: BinaryOutcome) -> usize {
    settlements.iter().filter(|s| s.outcome == outcome).count()
}

/// Brier score over decisive trades with predictions. The realized label is
/// "price finished above entry": a won call or lost put means up.
fn brier(settlements: &[BinarySettlement]) -> Option<f64> {
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for s in settlements {
        let (Some(p_up), Some(up)) = (s.position.signal.predicted_prob_up, went_up(s)) else {
            continue;
        };
        let label = if up { 1.0 } else { 0.0 };
        sum += (p_up - label).powi(2);
        n += 1;
    }
    (n > 0).then(|| sum / n as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::binary::{
        settle, BinaryAction, BinaryPosition, BinarySignal, TieBehavior,
    };
    use chrono::{DateTime, TimeZone, Utc};

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn settled(
        action: BinaryAction,
        entry: f64,
        exit: f64,
        prob_up: Option<f64>,
    ) -> BinarySettlement {
        let position = BinaryPosition {
            signal: BinarySignal {
                timestamp: t(0),
                action,
                stake: 1.0,
                expiry_seconds: 60,
                payout: 0.85,
                predicted_prob_up: prob_up,
                model_version: None,
                feature_hash: None,
                note: None,
            },
            entry_time: t(0),
            entry_price: entry,
        };
        settle(&position, exit, TieBehavior::RefundStake).unwrap()
    }

    fn call_win() -> BinarySettlement {
        settled(BinaryAction::BinaryCall, 1.0, 1.1, Some(0.6))
    }
    fn call_loss() -> BinarySettlement {
        settled(BinaryAction::BinaryCall, 1.0, 0.9, Some(0.6))
    }
    fn tie() -> BinarySettlement {
        settled(BinaryAction::BinaryCall, 1.0, 1.0, Some(0.6))
    }

    #[test]
    fn empty_report_has_no_rates() {
        let report = compute_binary_metrics(&[], 10);
        assert_eq!(report.trades, 0);
        assert_eq!(report.win_rate, None);
        assert_eq!(report.ev_per_trade, None);
        assert_eq!(report.coverage, Some(0.0));
    }

    #[test]
    fn win_rate_excludes_ties() {
        let report = compute_binary_metrics(&[call_win(), call_loss(), tie()], 0);
        assert_eq!(report.trades, 3);
        assert_eq!(report.ties, 1);
        assert!((report.win_rate.unwrap() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn edge_is_win_rate_minus_break_even() {
        // 2 wins, 1 loss at 0.85 payout: win rate 2/3, break-even ~0.5405.
        let report = compute_binary_metrics(&[call_win(), call_win(), call_loss()], 0);
        let edge = report.edge_over_break_even.unwrap();
        assert!((edge - (2.0 / 3.0 - 1.0 / 1.85)).abs() < 1e-9);
    }

    #[test]
    fn pnl_and_ev_match_payout_math() {
        // win +0.85, loss -1.0 -> net -0.15 over 2 trades.
        let report = compute_binary_metrics(&[call_win(), call_loss()], 0);
        assert!((report.net_pnl + 0.15).abs() < 1e-12);
        assert!((report.ev_per_trade.unwrap() + 0.075).abs() < 1e-12);
        assert!((report.total_staked - 2.0).abs() < 1e-12);
    }

    #[test]
    fn drawdown_tracks_peak_to_trough() {
        // +0.85, -1, -1: peak 0.85, trough -1.15 -> drawdown 2.0.
        let report =
            compute_binary_metrics(&[call_win(), call_loss(), call_loss()], 0);
        assert!((report.max_drawdown - 2.0).abs() < 1e-12);
    }

    #[test]
    fn losing_streak_counts_consecutive_losses_only() {
        let report = compute_binary_metrics(
            &[call_loss(), call_loss(), tie(), call_loss(), call_win()],
            0,
        );
        assert_eq!(report.longest_losing_streak, 2);
    }

    #[test]
    fn coverage_uses_opportunities() {
        let report = compute_binary_metrics(&[call_win(), call_loss()], 8);
        assert!((report.coverage.unwrap() - 0.25).abs() < 1e-12);
    }

    #[test]
    fn brier_scores_prediction_quality() {
        // Predicted 0.6 up. Win (up): (0.6-1)^2 = 0.16; loss (down): (0.6-0)^2 = 0.36.
        let report = compute_binary_metrics(&[call_win(), call_loss()], 0);
        assert!((report.brier_score.unwrap() - 0.26).abs() < 1e-9);
        // Ties and missing predictions are excluded.
        let no_pred = settled(BinaryAction::BinaryCall, 1.0, 1.1, None);
        let report = compute_binary_metrics(&[no_pred, tie()], 0);
        assert_eq!(report.brier_score, None);
    }

    #[test]
    fn grouped_metrics_split_by_key() {
        let by_outcome = compute_binary_metrics_by(&[call_win(), call_loss()], |s| {
            format!("{:?}", s.outcome)
        });
        assert_eq!(by_outcome.len(), 2);
        assert_eq!(by_outcome["Win"].wins, 1);
        assert_eq!(by_outcome["Loss"].losses, 1);
    }
}
