//! Promotion gates for binary-option models.
//!
//! The generic survivability check (positive P&L, drawdown < 35%, and a
//! 3-trade minimum) is far too weak for fixed-payout binaries, where a
//! positive stretch near break-even is routinely luck. A model is promotable
//! only when ALL gates pass:
//!
//! 1. Sample size: at least `min_trades` executed out-of-sample trades.
//! 2. Statistical edge: the Wilson lower confidence bound of the win rate
//!    must exceed the payout-adjusted break-even rate plus a safety margin.
//! 3. Positive expectancy: realized EV per trade must be positive.
//! 4. Drawdown: max drawdown bounded as a fraction of total staked.
//! 5. Payout stress: gate 2 must still hold after a payout haircut
//!    (payouts move; an edge that dies at -5 points of payout is not an edge).
//!
//! These gates cannot be relaxed by any automated process - changing them is
//! a reviewed code change by design.

use serde::{Deserialize, Serialize};

use crate::domain::binary::break_even_win_rate;
use crate::metrics::binary::BinaryMetricsReport;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BinaryPromotionGates {
    /// Minimum executed out-of-sample trades. Default 200.
    pub min_trades: usize,
    /// z-value of the one-sided confidence bound. Default 1.96 (~97.5%).
    pub confidence_z: f64,
    /// Wilson lower bound must exceed break-even by at least this. Default 0.
    pub min_edge_margin: f64,
    /// Max drawdown allowed, as a fraction of total staked. Default 0.25.
    pub max_drawdown_fraction: f64,
    /// Payout reduction applied for the stress re-check. Default 0.05.
    pub payout_haircut: f64,
}

impl Default for BinaryPromotionGates {
    fn default() -> Self {
        Self {
            min_trades: 200,
            confidence_z: 1.96,
            min_edge_margin: 0.0,
            max_drawdown_fraction: 0.25,
            payout_haircut: 0.05,
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

pub fn evaluate_promotion(
    report: &BinaryMetricsReport,
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

    let drawdown_cap = gates.max_drawdown_fraction * report.total_staked;
    check(
        "drawdown",
        report.total_staked > 0.0 && report.max_drawdown <= drawdown_cap,
        format!(
            "max drawdown {:.2} vs cap {:.2} ({:.0}% of {:.2} staked)",
            report.max_drawdown,
            drawdown_cap,
            gates.max_drawdown_fraction * 100.0,
            report.total_staked
        ),
    );

    PromotionVerdict {
        promotable: checks.iter().all(|c| c.passed),
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::binary::BinaryMetricsReport;

    fn report(wins: usize, losses: usize, payout: f64) -> BinaryMetricsReport {
        let trades = wins + losses;
        let net = wins as f64 * payout - losses as f64;
        BinaryMetricsReport {
            opportunities: trades * 2,
            trades,
            wins,
            losses,
            ties: 0,
            win_rate: (trades > 0).then(|| wins as f64 / trades as f64),
            avg_payout: (trades > 0).then_some(payout),
            break_even_win_rate: (trades > 0).then(|| break_even_win_rate(payout)),
            edge_over_break_even: None,
            total_staked: trades as f64,
            net_pnl: net,
            ev_per_trade: (trades > 0).then(|| net / trades as f64),
            max_drawdown: losses as f64 * 0.05, // mild, spread-out losses
            coverage: Some(0.5),
            longest_losing_streak: 3,
            brier_score: None,
        }
    }

    #[test]
    fn wilson_matches_known_value() {
        // 60/100 at z=1.96 -> ~0.502 (classic textbook value).
        let lb = wilson_lower_bound(60, 100, 1.96);
        assert!((lb - 0.502).abs() < 0.002, "{lb}");
        assert_eq!(wilson_lower_bound(0, 0, 1.96), 0.0);
        assert!(wilson_lower_bound(100, 100, 1.96) < 1.0);
    }

    #[test]
    fn small_profitable_sample_is_rejected() {
        // 20/30 at 0.85 payout is profitable but statistically nothing.
        let verdict = evaluate_promotion(&report(20, 10, 0.85), &BinaryPromotionGates::default());
        assert!(!verdict.promotable);
        let sample = verdict.checks.iter().find(|c| c.name == "sample_size").unwrap();
        assert!(!sample.passed);
    }

    #[test]
    fn barely_above_break_even_fails_statistical_gate() {
        // 56% over 500 trades at 0.85: above break-even (54.05%) but the
        // Wilson lower bound (~51.6%) is not.
        let verdict = evaluate_promotion(&report(280, 220, 0.85), &BinaryPromotionGates::default());
        assert!(!verdict.promotable);
        let edge = verdict.checks.iter().find(|c| c.name == "statistical_edge").unwrap();
        assert!(!edge.passed);
    }

    #[test]
    fn strong_large_sample_passes_all_gates() {
        // 60% over 1000 trades at 0.85: Wilson LB ~0.5695 > stressed
        // break-even 1/1.80 = 0.5556.
        let verdict = evaluate_promotion(&report(600, 400, 0.85), &BinaryPromotionGates::default());
        assert!(verdict.promotable, "{:?}", verdict.checks);
    }

    #[test]
    fn payout_haircut_kills_marginal_edges() {
        // 57% over 1500 trades at 0.85: Wilson LB ~0.5448 beats break-even
        // (0.5405) but not the stressed break-even (0.5556 at payout 0.80).
        let verdict = evaluate_promotion(&report(855, 645, 0.85), &BinaryPromotionGates::default());
        let edge = verdict.checks.iter().find(|c| c.name == "statistical_edge").unwrap();
        let stress = verdict.checks.iter().find(|c| c.name == "payout_stress").unwrap();
        assert!(edge.passed, "{}", edge.detail);
        assert!(!stress.passed, "{}", stress.detail);
        assert!(!verdict.promotable);
    }

    #[test]
    fn deep_drawdown_fails_even_with_edge() {
        let mut strong = report(600, 400, 0.85);
        strong.max_drawdown = 0.5 * strong.total_staked;
        let verdict = evaluate_promotion(&strong, &BinaryPromotionGates::default());
        let drawdown = verdict.checks.iter().find(|c| c.name == "drawdown").unwrap();
        assert!(!drawdown.passed);
        assert!(!verdict.promotable);
    }

    #[test]
    fn empty_report_fails_everything_gracefully() {
        let verdict = evaluate_promotion(&report(0, 0, 0.85), &BinaryPromotionGates::default());
        assert!(!verdict.promotable);
        assert_eq!(verdict.checks.len(), 5);
    }
}
