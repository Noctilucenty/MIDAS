//! Promotion gates for binary-option models.
//!
//! The generic survivability check (positive P&L, drawdown < 35%, and a
//! 3-trade minimum) is far too weak for fixed-payout binaries, where a
//! positive stretch near break-even is routinely luck. A model is promotable
//! only when ALL gates pass:
//!
//! 1.  Sample size: at least `min_trades` executed out-of-sample trades.
//! 2.  Statistical edge: the Wilson lower confidence bound of the win rate
//!     must exceed the payout-adjusted break-even rate plus a safety margin.
//! 3.  Payout stress: gate 2 must still hold after a payout haircut.
//! 4.  Positive expectancy: realized EV per trade must be positive.
//! 5.  Drawdown: max drawdown bounded as a fraction of a DECLARED starting
//!     bankroll (never of cumulative stake, which grows with trade count).
//! 6.  Calibration: Brier score must beat the class-prior constant predictor.
//! 7.  Fold consistency: enough walk-forward folds, and most of them
//!     individually non-negative.
//! 8.  Regime breadth: trades must span at least `min_days` distinct UTC days.
//! 9.  Payout provenance: results must rest on prospective payout snapshots,
//!     not assumed payouts.
//!
//! These gates cannot be relaxed by any automated process - changing them is
//! a reviewed code change by design.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::binary::{break_even_win_rate, BinarySettlement};
use crate::metrics::binary::BinaryMetricsReport;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryPromotionGates {
    /// Minimum executed out-of-sample trades. Default 200.
    pub min_trades: usize,
    /// z-value of the one-sided confidence bound. Default 1.96 (~97.5%).
    pub confidence_z: f64,
    /// Wilson lower bound must exceed break-even by at least this. Default 0.
    pub min_edge_margin: f64,
    /// DECLARED starting bankroll the drawdown cap is measured against.
    pub starting_bankroll: f64,
    /// Max drawdown allowed, as a fraction of starting bankroll. Default 0.25.
    pub max_drawdown_fraction: f64,
    /// Payout reduction applied for the stress re-check. Default 0.05.
    pub payout_haircut: f64,
    /// Minimum walk-forward folds represented in the settlements. Default 3.
    pub min_folds: usize,
    /// Share of folds that must have non-negative P&L. Default 0.6.
    pub min_fold_nonnegative_share: f64,
    /// Minimum distinct UTC days across trade entries. Default 5.
    pub min_days: usize,
    /// Whether payouts came from prospective snapshots (true) or were
    /// assumed (false). Assumed payouts can never promote.
    pub payout_source_prospective: bool,
}

impl Default for BinaryPromotionGates {
    fn default() -> Self {
        Self {
            min_trades: 200,
            confidence_z: 1.96,
            min_edge_margin: 0.0,
            starting_bankroll: 100.0,
            max_drawdown_fraction: 0.25,
            payout_haircut: 0.05,
            min_folds: 3,
            min_fold_nonnegative_share: 0.6,
            min_days: 5,
            payout_source_prospective: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromotionVerdict {
    pub promotable: bool,
    pub checks: Vec<GateCheck>,
}

/// Wilson score interval lower bound for a binomial proportion.
pub fn wilson_lower_bound(successes: usize, trials: usize, z: f64) -> f64 {
    if trials == 0 {
        return 0.0;
    }
    let n = trials as f64;
    let p = successes as f64 / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let center = p + z2 / (2.0 * n);
    let spread = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    ((center - spread) / denominator).max(0.0)
}

/// Extract the fold tag from a settlement's signal note ("fold=N").
fn fold_of(settlement: &BinarySettlement) -> Option<String> {
    settlement
        .position
        .signal
        .note
        .as_deref()
        .and_then(|note| note.split(',').find(|part| part.trim().starts_with("fold=")))
        .map(|part| part.trim().to_string())
}

pub fn evaluate_promotion(
    report: &BinaryMetricsReport,
    settlements: &[BinarySettlement],
    gates: &BinaryPromotionGates,
) -> PromotionVerdict {
    let mut checks = Vec::new();
    let mut check = |name: &str, passed: bool, detail: String| {
        checks.push(GateCheck {
            name: name.to_string(),
            passed,
            detail,
        });
    };

    check(
        "sample_size",
        report.trades >= gates.min_trades,
        format!("{} trades (need {})", report.trades, gates.min_trades),
    );

    let decisive = report.wins + report.losses;
    let lower = wilson_lower_bound(report.wins, decisive, gates.confidence_z);
    match report.avg_payout {
        Some(payout) => {
            let break_even = break_even_win_rate(payout);
            check(
                "statistical_edge",
                lower > break_even + gates.min_edge_margin,
                format!(
                    "wilson lower bound {:.4} vs break-even {:.4} + margin {:.4} (payout {:.2})",
                    lower, break_even, gates.min_edge_margin, payout
                ),
            );

            let stressed_payout = (payout - gates.payout_haircut).max(0.01);
            let stressed_break_even = break_even_win_rate(stressed_payout);
            check(
                "payout_stress",
                lower > stressed_break_even + gates.min_edge_margin,
                format!(
                    "wilson lower bound {:.4} vs stressed break-even {:.4} (payout {:.2} - {:.2})",
                    lower, stressed_break_even, payout, gates.payout_haircut
                ),
            );
        }
        None => {
            check("statistical_edge", false, "no trades - no payout data".to_string());
            check("payout_stress", false, "no trades - no payout data".to_string());
        }
    }

    check(
        "positive_expectancy",
        report.ev_per_trade.is_some_and(|ev| ev > 0.0),
        format!("ev_per_trade {:?}", report.ev_per_trade),
    );

    let drawdown_cap = gates.max_drawdown_fraction * gates.starting_bankroll;
    check(
        "drawdown",
        gates.starting_bankroll > 0.0 && report.max_drawdown <= drawdown_cap,
        format!(
            "max drawdown {:.2} vs cap {:.2} ({:.0}% of declared {:.2} bankroll)",
            report.max_drawdown,
            drawdown_cap,
            gates.max_drawdown_fraction * 100.0,
            gates.starting_bankroll
        ),
    );

    match (report.brier_score, report.label_up_rate) {
        (Some(brier), Some(prior)) => {
            let baseline = prior * (1.0 - prior);
            check(
                "calibration_vs_prior",
                brier < baseline,
                format!(
                    "brier {:.4} vs class-prior baseline {:.4} (up rate {:.3})",
                    brier, baseline, prior
                ),
            );
        }
        _ => check(
            "calibration_vs_prior",
            false,
            "missing predictions or no decisive trades".to_string(),
        ),
    }

    let mut fold_pnl: std::collections::BTreeMap<String, f64> = Default::default();
    for settlement in settlements {
        if let Some(fold) = fold_of(settlement) {
            *fold_pnl.entry(fold).or_insert(0.0) += settlement.pnl;
        }
    }
    let folds = fold_pnl.len();
    let nonnegative = fold_pnl.values().filter(|pnl| **pnl >= 0.0).count();
    let share = if folds > 0 {
        nonnegative as f64 / folds as f64
    } else {
        0.0
    };
    check(
        "fold_consistency",
        folds >= gates.min_folds && share >= gates.min_fold_nonnegative_share,
        format!(
            "{folds} folds (need {}), {nonnegative} non-negative ({:.0}% vs {:.0}% required)",
            gates.min_folds,
            share * 100.0,
            gates.min_fold_nonnegative_share * 100.0
        ),
    );

    let days: BTreeSet<String> = settlements
        .iter()
        .map(|s| s.position.entry_time.format("%Y-%m-%d").to_string())
        .collect();
    check(
        "regime_breadth",
        days.len() >= gates.min_days,
        format!("{} distinct UTC days (need {})", days.len(), gates.min_days),
    );

    check(
        "payout_provenance",
        gates.payout_source_prospective,
        if gates.payout_source_prospective {
            "prospective payout snapshots".to_string()
        } else {
            "ASSUMED payouts - cannot promote".to_string()
        },
    );

    PromotionVerdict {
        promotable: checks.iter().all(|c| c.passed),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::binary::{
        settle, BinaryAction, BinaryPosition, BinarySignal, TieBehavior,
    };
    use crate::metrics::binary::compute_binary_metrics;
    use chrono::{DateTime, Duration, TimeZone, Utc};

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    /// Build settlements: `wins` then `losses`, interleaved across `folds`
    /// fold tags and spread across `days` distinct days.
    fn make_settlements(
        wins: usize,
        losses: usize,
        payout: f64,
        folds: usize,
        days: usize,
    ) -> Vec<BinarySettlement> {
        let total = wins + losses;
        (0..total)
            .map(|i| {
                // Bresenham-style even interleave of wins and losses, so the
                // sequence carries no artificial losing streak.
                let win = (i + 1) * wins / total > i * wins / total;
                let position = BinaryPosition {
                    signal: BinarySignal {
                        timestamp: t(i as i64 * 60),
                        action: BinaryAction::BinaryCall,
                        stake: 1.0,
                        expiry_seconds: 60,
                        payout,
                        predicted_prob_up: Some(if win { 0.60 } else { 0.55 }),
                        model_version: None,
                        feature_hash: None,
                        note: Some(format!("fold={}", i % folds.max(1))),
                    },
                    entry_time: t(0) + Duration::days((i % days.max(1)) as i64)
                        + Duration::seconds(i as i64 * 60),
                    entry_price: 1.0,
                };
                settle(&position, if win { 1.1 } else { 0.9 }, TieBehavior::RefundStake).unwrap()
            })
            .collect()
    }

    fn promotable_gates() -> BinaryPromotionGates {
        BinaryPromotionGates {
            starting_bankroll: 100.0,
            payout_source_prospective: true,
            ..Default::default()
        }
    }

    #[test]
    fn wilson_matches_known_value() {
        let lb = wilson_lower_bound(60, 100, 1.96);
        assert!((lb - 0.502).abs() < 0.002, "{lb}");
        assert_eq!(wilson_lower_bound(0, 0, 1.96), 0.0);
        assert!(wilson_lower_bound(100, 100, 1.96) < 1.0);
    }

    #[test]
    fn strong_large_sample_passes_all_gates() {
        // 60% over 1000 trades at 0.85, 4 folds, 6 days, prospective payouts.
        let settlements = make_settlements(600, 400, 0.85, 4, 6);
        let report = compute_binary_metrics(&settlements, 2000);
        let verdict = evaluate_promotion(&report, &settlements, &promotable_gates());
        assert!(verdict.promotable, "{:#?}", verdict.checks);
    }

    #[test]
    fn assumed_payouts_can_never_promote() {
        let settlements = make_settlements(600, 400, 0.85, 4, 6);
        let report = compute_binary_metrics(&settlements, 2000);
        let gates = BinaryPromotionGates {
            payout_source_prospective: false,
            ..promotable_gates()
        };
        let verdict = evaluate_promotion(&report, &settlements, &gates);
        assert!(!verdict.promotable);
        let provenance = verdict.checks.iter().find(|c| c.name == "payout_provenance").unwrap();
        assert!(!provenance.passed);
    }

    #[test]
    fn small_profitable_sample_is_rejected() {
        let settlements = make_settlements(20, 10, 0.85, 4, 6);
        let report = compute_binary_metrics(&settlements, 60);
        let verdict = evaluate_promotion(&report, &settlements, &promotable_gates());
        assert!(!verdict.promotable);
        let sample = verdict.checks.iter().find(|c| c.name == "sample_size").unwrap();
        assert!(!sample.passed);
    }

    #[test]
    fn barely_above_break_even_fails_statistical_gate() {
        // 56% over 500: above break-even but Wilson LB is not.
        let settlements = make_settlements(280, 220, 0.85, 4, 6);
        let report = compute_binary_metrics(&settlements, 1000);
        let verdict = evaluate_promotion(&report, &settlements, &promotable_gates());
        let edge = verdict.checks.iter().find(|c| c.name == "statistical_edge").unwrap();
        assert!(!edge.passed);
        assert!(!verdict.promotable);
    }

    #[test]
    fn payout_haircut_kills_marginal_edges() {
        // 57% over 1500: Wilson LB ~0.5448 beats break-even (0.5405) but not
        // the stressed break-even (0.5556 at payout 0.80).
        let settlements = make_settlements(855, 645, 0.85, 4, 6);
        let report = compute_binary_metrics(&settlements, 3000);
        let verdict = evaluate_promotion(&report, &settlements, &promotable_gates());
        let edge = verdict.checks.iter().find(|c| c.name == "statistical_edge").unwrap();
        let stress = verdict.checks.iter().find(|c| c.name == "payout_stress").unwrap();
        assert!(edge.passed, "{}", edge.detail);
        assert!(!stress.passed, "{}", stress.detail);
        assert!(!verdict.promotable);
    }

    #[test]
    fn drawdown_is_measured_against_declared_bankroll() {
        let settlements = make_settlements(600, 400, 0.85, 4, 6);
        let report = compute_binary_metrics(&settlements, 2000);
        // Same trades, tiny declared bankroll (cap 0.5 vs ~1.0 drawdown):
        // the identical drawdown must now breach.
        let gates = BinaryPromotionGates {
            starting_bankroll: 2.0,
            ..promotable_gates()
        };
        let verdict = evaluate_promotion(&report, &settlements, &gates);
        let drawdown = verdict.checks.iter().find(|c| c.name == "drawdown").unwrap();
        assert!(!drawdown.passed, "{}", drawdown.detail);
    }

    #[test]
    fn too_few_days_fails_regime_breadth() {
        let settlements = make_settlements(600, 400, 0.85, 4, 2);
        let report = compute_binary_metrics(&settlements, 2000);
        let verdict = evaluate_promotion(&report, &settlements, &promotable_gates());
        let breadth = verdict.checks.iter().find(|c| c.name == "regime_breadth").unwrap();
        assert!(!breadth.passed);
    }

    #[test]
    fn too_few_folds_fails_consistency() {
        let settlements = make_settlements(600, 400, 0.85, 1, 6);
        let report = compute_binary_metrics(&settlements, 2000);
        let verdict = evaluate_promotion(&report, &settlements, &promotable_gates());
        let folds = verdict.checks.iter().find(|c| c.name == "fold_consistency").unwrap();
        assert!(!folds.passed);
    }

    #[test]
    fn poor_calibration_fails_against_prior_baseline() {
        // Wins predicted at 0.55, losses at 0.60: inverted, worse than prior.
        let mut settlements = make_settlements(600, 400, 0.85, 4, 6);
        for s in &mut settlements {
            s.position.signal.predicted_prob_up = Some(match s.outcome {
                crate::domain::binary::BinaryOutcome::Win => 0.10,
                _ => 0.90,
            });
        }
        let report = compute_binary_metrics(&settlements, 2000);
        let verdict = evaluate_promotion(&report, &settlements, &promotable_gates());
        let calibration = verdict.checks.iter().find(|c| c.name == "calibration_vs_prior").unwrap();
        assert!(!calibration.passed, "{}", calibration.detail);
    }

    #[test]
    fn empty_report_fails_everything_gracefully() {
        let report = compute_binary_metrics(&[], 0);
        let verdict = evaluate_promotion(&report, &[], &promotable_gates());
        assert!(!verdict.promotable);
        assert_eq!(verdict.checks.len(), 9);
    }
}
