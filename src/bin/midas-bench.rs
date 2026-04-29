use std::time::Instant;

use chrono::{Duration, TimeZone, Utc};
use clap::Parser;
use midas_backtesting_engine::domain::config::{
    BacktestConfig, BacktestRequest, ExecutionConfig, LimitFillAssumption, LiquidationConfig,
    MarketPriceReference, OrderTypeAssumption, ParameterSweep, ParameterValue, PositionSizing,
    RunContext, StrategyDefinition, ValidationConfig, ValidationRequest,
};
use midas_backtesting_engine::domain::types::{
    Candle, MarketDataSet, MarketEvent, MarketEventKind, Signal, SignalAction, StrategyInput,
};
use midas_backtesting_engine::engine::backtester::BacktestEngine;
use midas_backtesting_engine::validation::run_validation;

#[derive(Parser, Debug)]
#[command(
    name = "midas-bench",
    about = "Lightweight benchmark harness for MIDAS core paths"
)]
struct Cli {
    #[arg(long, default_value_t = 50_000)]
    candles: usize,
    #[arg(long, default_value_t = 100_000)]
    events: usize,
    #[arg(long, default_value_t = 6)]
    sweep_values: usize,
}

fn main() {
    let cli = Cli::parse();

    let candle_request = candle_backtest_request(cli.candles);
    let event_request = event_backtest_request(cli.events);
    let validation_request = validation_request(cli.candles.max(64), cli.sweep_values);

    let candle_start = Instant::now();
    let candle_report = BacktestEngine::run(candle_request, 0).expect("candle benchmark failed");
    let candle_elapsed = candle_start.elapsed();

    let event_start = Instant::now();
    let event_report = BacktestEngine::run(event_request, 0).expect("event benchmark failed");
    let event_elapsed = event_start.elapsed();

    let validation_start = Instant::now();
    let validation_report =
        run_validation(validation_request).expect("validation benchmark failed");
    let validation_elapsed = validation_start.elapsed();

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "candle_backtest": {
                "observations": cli.candles,
                "elapsed_ms": candle_elapsed.as_secs_f64() * 1000.0,
                "observations_per_second": cli.candles as f64 / candle_elapsed.as_secs_f64().max(1e-9),
                "trade_count": candle_report.metrics.trade_count,
            },
            "event_backtest": {
                "observations": cli.events,
                "elapsed_ms": event_elapsed.as_secs_f64() * 1000.0,
                "observations_per_second": cli.events as f64 / event_elapsed.as_secs_f64().max(1e-9),
                "trade_count": event_report.metrics.trade_count,
            },
            "validation": {
                "observations": cli.candles.max(64),
                "parameter_sweep_values": cli.sweep_values,
                "elapsed_ms": validation_elapsed.as_secs_f64() * 1000.0,
                "score": validation_report.summary.score,
                "parameter_runs": validation_report.parameter_sensitivity.len(),
            }
        }))
        .unwrap()
    );
}

fn candle_backtest_request(count: usize) -> BacktestRequest {
    let candles = (0..count)
        .map(|index| {
            let base = 100.0 + (index % 500) as f64 * 0.1;
            Candle {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                    + Duration::minutes(index as i64),
                open: base,
                high: base + 0.8,
                low: base - 0.8,
                close: base + if index % 2 == 0 { 0.3 } else { -0.15 },
                volume: 10.0 + index as f64 % 5.0,
                funding_rate: if index % 480 == 0 { 0.0001 } else { 0.0 },
                spread_bps: Some(2.0),
            }
        })
        .collect();
    BacktestRequest {
        context: base_context("benchmark_candles", "1m"),
        market_data: MarketDataSet::Candles(candles),
        strategy_input: StrategyInput::Definition(StrategyDefinition::MovingAverageCross {
            fast_window: 5,
            slow_window: 20,
        }),
        backtest_config: base_backtest_config(),
        execution_config: base_execution_config(OrderTypeAssumption::Market),
    }
}

fn event_backtest_request(count: usize) -> BacktestRequest {
    let base = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    let events = (0..count)
        .map(|index| {
            let price = 100.0 + (index % 1_000) as f64 * 0.01;
            MarketEvent {
                timestamp: base + Duration::milliseconds(index as i64),
                sequence: index as u64 + 1,
                kind: match index % 4 {
                    0 => MarketEventKind::Quote {
                        bid: price - 0.05,
                        ask: price + 0.05,
                        bid_size: Some(1.0),
                        ask_size: Some(1.0),
                    },
                    1 => MarketEventKind::Trade {
                        price,
                        quantity: 0.5,
                        aggressor: None,
                    },
                    2 => MarketEventKind::Funding { rate: 0.0 },
                    _ => MarketEventKind::Depth {
                        side: if index % 8 == 0 {
                            midas_backtesting_engine::domain::types::Side::Long
                        } else {
                            midas_backtesting_engine::domain::types::Side::Short
                        },
                        price,
                        quantity: 1.0,
                        level: Some(1),
                        action: midas_backtesting_engine::domain::types::OrderBookAction::Upsert,
                    },
                },
            }
        })
        .collect();
    BacktestRequest {
        context: base_context("benchmark_events", "event"),
        market_data: MarketDataSet::Events(events),
        strategy_input: StrategyInput::SignalStream(vec![
            Signal {
                timestamp: base,
                action: SignalAction::GoLong,
                leverage_override: Some(1.0),
                limit_price: None,
                note: Some("bench_long".to_string()),
            },
            Signal {
                timestamp: base + Duration::milliseconds((count / 2) as i64),
                action: SignalAction::GoShort,
                leverage_override: Some(1.0),
                limit_price: None,
                note: Some("bench_flip".to_string()),
            },
        ]),
        backtest_config: base_backtest_config(),
        execution_config: {
            let mut config = base_execution_config(OrderTypeAssumption::Market);
            config.use_candle_spread = false;
            config.market_price_reference = MarketPriceReference::LastTrade;
            config
        },
    }
}

fn validation_request(count: usize, sweep_values: usize) -> ValidationRequest {
    let mut candles_request = candle_backtest_request(count);
    candles_request.context.run_label = Some("benchmark_validation".to_string());
    ValidationRequest {
        backtest_request: candles_request,
        validation_config: ValidationConfig {
            in_sample_ratio: 0.6,
            stress_scenarios: vec![],
            parameter_sweeps: vec![ParameterSweep {
                name: "fast_window".to_string(),
                values: (2..(2 + sweep_values))
                    .map(|value| ParameterValue::Int(value as i64))
                    .collect(),
            }],
            walk_forward: None,
            regime_windows: vec![],
            deterministic_seed: 0,
            min_trades_for_score: 1,
        },
    }
}

fn base_context(label: &str, timeframe: &str) -> RunContext {
    RunContext {
        symbol: "BTC-PERP".to_string(),
        venue: Some("bench".to_string()),
        timeframe: timeframe.to_string(),
        run_label: Some(label.to_string()),
    }
}

fn base_backtest_config() -> BacktestConfig {
    BacktestConfig {
        starting_cash: 10_000.0,
        default_leverage: 1.5,
        max_leverage: 3.0,
        position_sizing: PositionSizing::FixedNotional { notional: 500.0 },
        allow_long: true,
        allow_short: true,
    }
}

fn base_execution_config(order_type: OrderTypeAssumption) -> ExecutionConfig {
    ExecutionConfig {
        taker_fee_bps: 5.0,
        maker_fee_bps: 2.0,
        spread_bps: 2.0,
        slippage_bps: 1.0,
        latency_bars: 0,
        latency_events: 0,
        order_timeout_bars: Some(2),
        order_timeout_events: Some(4),
        order_type,
        market_price_reference: MarketPriceReference::OpposingBest,
        limit_fill_assumption: LimitFillAssumption::Touch,
        use_candle_spread: true,
        partial_fill_ratio: None,
        liquidation: LiquidationConfig::default(),
    }
}
