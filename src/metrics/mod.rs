use crate::domain::types::{DrawdownPoint, EquityPoint, MetricsReport, Trade};

pub fn compute_metrics(
    starting_cash: f64,
    equity_curve: &[EquityPoint],
    drawdown_curve: &[DrawdownPoint],
    trades: &[Trade],
    fee_impact: f64,
    funding_impact: f64,
    slippage_impact: f64,
    spread_impact: f64,
    exposed_bars: usize,
) -> MetricsReport {
    let ending_equity = equity_curve
        .last()
        .map(|p| p.equity)
        .unwrap_or(starting_cash);
    let net_pnl = ending_equity - starting_cash;
    let gross_pnl: f64 = trades.iter().map(|trade| trade.gross_pnl).sum();
    let wins: Vec<&Trade> = trades.iter().filter(|trade| trade.net_pnl > 0.0).collect();
    let losses: Vec<&Trade> = trades.iter().filter(|trade| trade.net_pnl < 0.0).collect();
    let profit_factor = {
        let gross_profit: f64 = wins.iter().map(|trade| trade.net_pnl).sum();
        let gross_loss: f64 = losses.iter().map(|trade| trade.net_pnl.abs()).sum();
        if gross_loss > 0.0 {
            gross_profit / gross_loss
        } else if gross_profit > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    };
    let average_win = if wins.is_empty() {
        0.0
    } else {
        wins.iter().map(|trade| trade.net_pnl).sum::<f64>() / wins.len() as f64
    };
    let average_loss = if losses.is_empty() {
        0.0
    } else {
        losses.iter().map(|trade| trade.net_pnl).sum::<f64>() / losses.len() as f64
    };
    let trade_count = trades.len();
    let exposure_time_pct = if equity_curve.is_empty() {
        0.0
    } else {
        exposed_bars as f64 / equity_curve.len() as f64
    };
    let average_trade_duration_seconds = if trades.is_empty() {
        0.0
    } else {
        trades
            .iter()
            .map(|trade| trade.duration_seconds as f64)
            .sum::<f64>()
            / trades.len() as f64
    };
    let max_drawdown = drawdown_curve
        .iter()
        .map(|point| point.drawdown)
        .fold(0.0_f64, f64::max);
    let max_drawdown_pct = drawdown_curve
        .iter()
        .map(|point| point.drawdown_pct)
        .fold(0.0_f64, f64::max);

    MetricsReport {
        total_return: if starting_cash > 0.0 {
            net_pnl / starting_cash
        } else {
            0.0
        },
        net_pnl,
        gross_pnl,
        sharpe_ratio: sharpe_ratio(equity_curve),
        max_drawdown,
        max_drawdown_pct,
        win_rate: if trade_count > 0 {
            wins.len() as f64 / trade_count as f64
        } else {
            0.0
        },
        profit_factor,
        average_win,
        average_loss,
        trade_count,
        exposure_time_pct,
        average_trade_duration_seconds,
        funding_cost_impact: funding_impact,
        fee_impact,
        slippage_impact,
        spread_impact,
        ending_equity,
    }
}

pub fn sharpe_ratio(equity_curve: &[EquityPoint]) -> f64 {
    if equity_curve.len() < 3 {
        return 0.0;
    }
    let returns: Vec<f64> = equity_curve
        .windows(2)
        .filter_map(|window| {
            let previous = window[0].equity;
            let current = window[1].equity;
            if previous.abs() < f64::EPSILON {
                None
            } else {
                Some((current / previous) - 1.0)
            }
        })
        .collect();
    if returns.len() < 2 {
        return 0.0;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| {
            let diff = value - mean;
            diff * diff
        })
        .sum::<f64>()
        / returns.len() as f64;
    let std_dev = variance.sqrt();
    if std_dev.abs() < f64::EPSILON {
        return 0.0;
    }

    let median_delta_seconds = {
        let mut deltas: Vec<i64> = equity_curve
            .windows(2)
            .map(|window| {
                window[1]
                    .timestamp
                    .signed_duration_since(window[0].timestamp)
                    .num_seconds()
            })
            .filter(|delta| *delta > 0)
            .collect();
        if deltas.is_empty() {
            return 0.0;
        }
        deltas.sort_unstable();
        deltas[deltas.len() / 2] as f64
    };

    let periods_per_year = (365.0 * 24.0 * 60.0 * 60.0) / median_delta_seconds;
    if !periods_per_year.is_finite() || periods_per_year <= 0.0 {
        return 0.0;
    }
    (mean / std_dev) * periods_per_year.sqrt()
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::{compute_metrics, sharpe_ratio};
    use crate::domain::types::{DrawdownPoint, EquityPoint, PositionState, Side, Trade};

    fn point(offset_hours: i64, equity: f64) -> EquityPoint {
        EquityPoint {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                + Duration::hours(offset_hours),
            equity,
            cash: equity,
            unrealized_pnl: 0.0,
            position_state: PositionState::Flat,
        }
    }

    #[test]
    fn sharpe_ratio_is_positive_for_upward_curve() {
        let equity = vec![
            point(0, 100.0),
            point(1, 101.0),
            point(2, 102.5),
            point(3, 103.0),
        ];
        assert!(sharpe_ratio(&equity) > 0.0);
    }

    #[test]
    fn max_drawdown_is_exposed_in_metrics() {
        let equity_curve = vec![point(0, 100.0), point(1, 120.0), point(2, 90.0)];
        let drawdown_curve = vec![
            DrawdownPoint {
                timestamp: equity_curve[0].timestamp,
                equity: 100.0,
                peak_equity: 100.0,
                drawdown: 0.0,
                drawdown_pct: 0.0,
            },
            DrawdownPoint {
                timestamp: equity_curve[1].timestamp,
                equity: 120.0,
                peak_equity: 120.0,
                drawdown: 0.0,
                drawdown_pct: 0.0,
            },
            DrawdownPoint {
                timestamp: equity_curve[2].timestamp,
                equity: 90.0,
                peak_equity: 120.0,
                drawdown: 30.0,
                drawdown_pct: 0.25,
            },
        ];
        let metrics = compute_metrics(
            100.0,
            &equity_curve,
            &drawdown_curve,
            &[],
            0.0,
            0.0,
            0.0,
            0.0,
            0,
        );
        assert!((metrics.max_drawdown - 30.0).abs() < 1e-9);
        assert!((metrics.max_drawdown_pct - 0.25).abs() < 1e-9);
    }

    #[test]
    fn average_trade_duration_is_reported() {
        let trade = Trade {
            entry_time: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            exit_time: Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap(),
            side: Side::Long,
            quantity: 1.0,
            leverage: 1.0,
            entry_price: 100.0,
            exit_price: 105.0,
            gross_pnl: 5.0,
            net_pnl: 4.0,
            fees_paid: 0.5,
            funding_paid: 0.0,
            slippage_paid: 0.25,
            spread_paid: 0.25,
            duration_seconds: 7_200,
            liquidated: false,
            entry_note: None,
            exit_note: None,
        };
        let metrics = compute_metrics(
            100.0,
            &[point(0, 100.0), point(1, 104.0)],
            &[DrawdownPoint {
                timestamp: point(0, 100.0).timestamp,
                equity: 100.0,
                peak_equity: 100.0,
                drawdown: 0.0,
                drawdown_pct: 0.0,
            }],
            &[trade],
            0.5,
            0.0,
            0.25,
            0.25,
            1,
        );
        assert_eq!(metrics.average_trade_duration_seconds, 7_200.0);
    }
}
