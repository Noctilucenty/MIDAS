use crate::domain::errors::BacktestError;
use crate::domain::types::{
    ConsistencyCheckResult, ConsistencyReport, DrawdownPoint, EquityPoint, MetricsReport,
    OrderStatus, PositionState,
};
use crate::engine::state::BacktestState;

const EPSILON: f64 = 1e-8;

pub fn check_backtest_consistency(
    starting_cash: f64,
    state: &BacktestState,
    equity_curve: &[EquityPoint],
    drawdown_curve: &[DrawdownPoint],
    metrics: &MetricsReport,
) -> Result<ConsistencyReport, BacktestError> {
    let mut report = ConsistencyReport { checks: Vec::new() };

    let last_equity = equity_curve
        .last()
        .ok_or_else(|| BacktestError::InvariantViolation {
            name: "equity_curve_non_empty".to_string(),
            detail: "equity curve is empty".to_string(),
        })?;
    let last_drawdown = drawdown_curve
        .last()
        .ok_or_else(|| BacktestError::InvariantViolation {
            name: "drawdown_curve_non_empty".to_string(),
            detail: "drawdown curve is empty".to_string(),
        })?;

    push_check(
        &mut report,
        "equity_points_reconcile",
        equity_curve
            .iter()
            .all(|point| approx_eq(point.equity, point.cash + point.unrealized_pnl)),
        "each equity point must equal cash plus unrealized pnl",
    );
    push_check(
        &mut report,
        "flat_points_have_zero_unrealized",
        equity_curve.iter().all(|point| {
            point.position_state != PositionState::Flat || approx_eq(point.unrealized_pnl, 0.0)
        }),
        "flat equity points must not carry unrealized pnl",
    );
    push_check(
        &mut report,
        "drawdown_points_non_negative",
        drawdown_curve
            .iter()
            .all(|point| point.drawdown >= -EPSILON && point.drawdown_pct >= -EPSILON),
        "drawdown values must be non-negative distances from peak equity",
    );
    push_check(
        &mut report,
        "drawdown_points_match_running_peak",
        drawdown_curve.windows(2).all(|window| {
            window[1].peak_equity + EPSILON >= window[0].peak_equity
                && window[1].peak_equity + EPSILON >= window[1].equity
        }),
        "drawdown peak equity must be monotonic and above current equity",
    );
    push_check(
        &mut report,
        "trade_count_matches_metrics",
        metrics.trade_count == state.trades.len(),
        format!(
            "metrics trade_count={} but trade_log has {} entries",
            metrics.trade_count,
            state.trades.len()
        ),
    );
    push_check(
        &mut report,
        "ending_equity_matches_last_point",
        approx_eq(metrics.ending_equity, last_equity.equity),
        format!(
            "metrics ending_equity={} but last equity point={}",
            metrics.ending_equity, last_equity.equity
        ),
    );
    push_check(
        &mut report,
        "net_pnl_matches_start_to_end_equity",
        approx_eq(metrics.net_pnl, metrics.ending_equity - starting_cash),
        format!(
            "metrics net_pnl={} but ending_equity-starting_cash={}",
            metrics.net_pnl,
            metrics.ending_equity - starting_cash
        ),
    );

    let open_fee_carry = state
        .position
        .as_ref()
        .map(|position| position.accumulated_fees)
        .unwrap_or(0.0);
    let open_funding_carry = state
        .position
        .as_ref()
        .map(|position| position.accumulated_funding)
        .unwrap_or(0.0);
    let open_slippage_carry = state
        .position
        .as_ref()
        .map(|position| position.accumulated_slippage)
        .unwrap_or(0.0);
    let open_spread_carry = state
        .position
        .as_ref()
        .map(|position| position.accumulated_spread)
        .unwrap_or(0.0);
    let trade_fees: f64 = state.trades.iter().map(|trade| trade.fees_paid).sum();
    let trade_funding: f64 = state.trades.iter().map(|trade| trade.funding_paid).sum();
    let trade_slippage: f64 = state.trades.iter().map(|trade| trade.slippage_paid).sum();
    let trade_spread: f64 = state.trades.iter().map(|trade| trade.spread_paid).sum();

    push_check(
        &mut report,
        "fee_impact_reconciles",
        approx_eq(state.fee_impact, trade_fees + open_fee_carry),
        format!(
            "state fee_impact={} but trades + open carry={}",
            state.fee_impact,
            trade_fees + open_fee_carry
        ),
    );
    push_check(
        &mut report,
        "funding_impact_reconciles",
        approx_eq(state.funding_impact, trade_funding + open_funding_carry),
        format!(
            "state funding_impact={} but trades + open carry={}",
            state.funding_impact,
            trade_funding + open_funding_carry
        ),
    );
    push_check(
        &mut report,
        "slippage_impact_reconciles",
        approx_eq(state.slippage_impact, trade_slippage + open_slippage_carry),
        format!(
            "state slippage_impact={} but trades + open carry={}",
            state.slippage_impact,
            trade_slippage + open_slippage_carry
        ),
    );
    push_check(
        &mut report,
        "spread_impact_reconciles",
        approx_eq(state.spread_impact, trade_spread + open_spread_carry),
        format!(
            "state spread_impact={} but trades + open carry={}",
            state.spread_impact,
            trade_spread + open_spread_carry
        ),
    );
    push_check(
        &mut report,
        "diagnostic_costs_match_metrics",
        approx_eq(metrics.fee_impact, state.fee_impact)
            && approx_eq(metrics.funding_cost_impact, state.funding_impact)
            && approx_eq(metrics.slippage_impact, state.slippage_impact)
            && approx_eq(metrics.spread_impact, state.spread_impact),
        "metrics cost attribution fields must match state totals",
    );
    push_check(
        &mut report,
        "execution_legs_match_fills",
        state.execution_legs.len() == state.fills.len()
            && state
                .execution_legs
                .iter()
                .zip(&state.fills)
                .all(|(leg, fill)| {
                    leg.order_id == fill.order_id
                        && leg.side == fill.side
                        && approx_eq(leg.quantity, fill.quantity)
                        && approx_eq(leg.realized_execution_price, fill.realized_execution_price)
                }),
        "execution legs must line up one-for-one with fills",
    );
    push_check(
        &mut report,
        "close_legs_match_closed_trades",
        state
            .execution_legs
            .iter()
            .filter(|leg| leg.role != crate::domain::types::ExecutionLegRole::Open)
            .count()
            == state.trades.len(),
        "every closed trade must map to exactly one close or liquidation leg",
    );
    push_check(
        &mut report,
        "order_lifecycle_entries_are_legal",
        orders_have_legal_lifecycle(state),
        "each non-liquidation order must begin with submitted and end in a single terminal state",
    );
    push_check(
        &mut report,
        "final_position_matches_last_equity_state",
        match &state.position {
            Some(position) => last_equity.position_state == position.state(),
            None => last_equity.position_state == PositionState::Flat,
        },
        "final position state must match the last equity point",
    );
    push_check(
        &mut report,
        "final_drawdown_matches_last_equity",
        approx_eq(last_drawdown.equity, last_equity.equity)
            && last_drawdown.peak_equity + EPSILON >= last_drawdown.equity,
        "last drawdown point must reconcile with the last equity point",
    );

    if let Some(failed) = report.checks.iter().find(|check| !check.passed) {
        return Err(BacktestError::InvariantViolation {
            name: failed.name.clone(),
            detail: failed.detail.clone(),
        });
    }

    Ok(report)
}

fn push_check(report: &mut ConsistencyReport, name: &str, passed: bool, detail: impl Into<String>) {
    report.checks.push(ConsistencyCheckResult {
        name: name.to_string(),
        passed,
        detail: detail.into(),
    });
}

fn orders_have_legal_lifecycle(state: &BacktestState) -> bool {
    let mut grouped: std::collections::BTreeMap<u64, Vec<&crate::domain::types::OrderAuditEntry>> =
        std::collections::BTreeMap::new();
    for entry in &state.order_audit_log {
        grouped.entry(entry.order_id).or_default().push(entry);
    }

    grouped.into_iter().all(|(order_id, entries)| {
        if order_id == 0 {
            return entries
                .iter()
                .all(|entry| matches!(entry.status, OrderStatus::Filled));
        }

        let Some(first) = entries.first() else {
            return false;
        };
        if !matches!(first.status, OrderStatus::Submitted) {
            return false;
        }
        let terminal = entries
            .iter()
            .filter(|entry| !matches!(entry.status, OrderStatus::Submitted))
            .collect::<Vec<_>>();
        if terminal.len() != 1 {
            return false;
        }
        matches!(
            terminal[0].status,
            OrderStatus::Filled
                | OrderStatus::Cancelled
                | OrderStatus::Expired
                | OrderStatus::Rejected
        )
    })
}

fn approx_eq(left: f64, right: f64) -> bool {
    (left - right).abs() <= EPSILON.max(left.abs().max(right.abs()) * 1e-9)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::check_backtest_consistency;
    use crate::domain::errors::BacktestError;
    use crate::domain::types::{
        ConsistencyReport, DrawdownPoint, EquityPoint, ExecutionDiagnostics, MetricsReport,
        PositionState, ReplayDiagnostics,
    };
    use crate::engine::state::BacktestState;

    fn sample_state() -> BacktestState {
        BacktestState {
            cash: 101.0,
            execution_diagnostics: ExecutionDiagnostics::default(),
            replay_diagnostics: ReplayDiagnostics::default(),
            ..BacktestState::default()
        }
    }

    fn sample_metrics() -> MetricsReport {
        MetricsReport {
            total_return: 0.01,
            net_pnl: 1.0,
            gross_pnl: 0.0,
            sharpe_ratio: 0.0,
            max_drawdown: 0.0,
            max_drawdown_pct: 0.0,
            win_rate: 0.0,
            profit_factor: 0.0,
            average_win: 0.0,
            average_loss: 0.0,
            trade_count: 0,
            exposure_time_pct: 0.0,
            average_trade_duration_seconds: 0.0,
            funding_cost_impact: 0.0,
            fee_impact: 0.0,
            slippage_impact: 0.0,
            spread_impact: 0.0,
            ending_equity: 101.0,
        }
    }

    #[test]
    fn invariant_checker_accepts_consistent_terminal_state() {
        let timestamp = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let equity = vec![EquityPoint {
            timestamp,
            equity: 101.0,
            cash: 101.0,
            unrealized_pnl: 0.0,
            position_state: PositionState::Flat,
        }];
        let drawdown = vec![DrawdownPoint {
            timestamp,
            equity: 101.0,
            peak_equity: 101.0,
            drawdown: 0.0,
            drawdown_pct: 0.0,
        }];
        let report = check_backtest_consistency(
            100.0,
            &sample_state(),
            &equity,
            &drawdown,
            &sample_metrics(),
        )
        .unwrap();
        assert!(matches!(report, ConsistencyReport { .. }));
        assert!(report.checks.iter().all(|check| check.passed));
    }

    #[test]
    fn invariant_checker_rejects_broken_equity_identity() {
        let timestamp = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap() + Duration::hours(1);
        let equity = vec![EquityPoint {
            timestamp,
            equity: 101.0,
            cash: 100.0,
            unrealized_pnl: 0.5,
            position_state: PositionState::Flat,
        }];
        let drawdown = vec![DrawdownPoint {
            timestamp,
            equity: 101.0,
            peak_equity: 101.0,
            drawdown: 0.0,
            drawdown_pct: 0.0,
        }];
        let error = check_backtest_consistency(
            100.0,
            &sample_state(),
            &equity,
            &drawdown,
            &sample_metrics(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            BacktestError::InvariantViolation { name, .. } if name == "equity_points_reconcile"
        ));
    }
}
