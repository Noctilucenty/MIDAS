use chrono::{Duration, TimeZone, Utc};

use midas_backtesting_engine::domain::config::{
    BacktestConfig, BacktestRequest, ExecutionConfig, LimitFillAssumption, LiquidationConfig,
    MarketPriceReference, OrderTypeAssumption, PositionSizing, RunContext, StrategyDefinition,
    ValidationConfig, ValidationRequest,
};
use midas_backtesting_engine::domain::types::{
    Candle, MarketDataSet, MarketEvent, MarketEventKind, Signal, StrategyInput,
};

pub fn sample_candles(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|index| {
            let base = 100.0 + index as f64;
            Candle {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                    + Duration::hours(index as i64),
                open: base,
                high: base + 1.0,
                low: base - 1.0,
                close: base + if index % 2 == 0 { 0.6 } else { -0.2 },
                volume: 100.0 + index as f64,
                funding_rate: if index % 4 == 0 { 0.0001 } else { 0.0 },
                spread_bps: Some(2.0),
            }
        })
        .collect()
}

pub fn sample_execution_config(order_type: OrderTypeAssumption) -> ExecutionConfig {
    ExecutionConfig {
        taker_fee_bps: 5.0,
        maker_fee_bps: 2.0,
        spread_bps: 2.0,
        slippage_bps: 1.0,
        latency_bars: 0,
        latency_events: 0,
        order_timeout_bars: Some(1),
        order_timeout_events: Some(1),
        order_type,
        market_price_reference: MarketPriceReference::OpposingBest,
        limit_fill_assumption: LimitFillAssumption::Touch,
        use_candle_spread: true,
        partial_fill_ratio: None,
        liquidation: LiquidationConfig::default(),
    }
}

pub fn signal_stream_request(signals: Vec<Signal>, execution: ExecutionConfig) -> BacktestRequest {
    BacktestRequest {
        context: RunContext {
            symbol: "BTC-PERP".to_string(),
            venue: Some("test".to_string()),
            timeframe: "1h".to_string(),
            run_label: Some("test".to_string()),
        },
        market_data: MarketDataSet::Candles(sample_candles(8)),
        strategy_input: StrategyInput::SignalStream(signals),
        backtest_config: BacktestConfig {
            starting_cash: 1_000.0,
            default_leverage: 1.5,
            max_leverage: 3.0,
            position_sizing: PositionSizing::FixedNotional { notional: 300.0 },
            allow_long: true,
            allow_short: true,
        },
        execution_config: execution,
    }
}

pub fn validation_request() -> ValidationRequest {
    ValidationRequest {
        backtest_request: BacktestRequest {
            context: RunContext {
                symbol: "BTC-PERP".to_string(),
                venue: Some("test".to_string()),
                timeframe: "1h".to_string(),
                run_label: Some("golden".to_string()),
            },
            market_data: MarketDataSet::Candles(sample_candles(12)),
            strategy_input: StrategyInput::Definition(StrategyDefinition::MovingAverageCross {
                fast_window: 2,
                slow_window: 4,
            }),
            backtest_config: BacktestConfig {
                starting_cash: 1_000.0,
                default_leverage: 1.5,
                max_leverage: 2.0,
                position_sizing: PositionSizing::FixedNotional { notional: 400.0 },
                allow_long: true,
                allow_short: true,
            },
            execution_config: sample_execution_config(OrderTypeAssumption::Market),
        },
        validation_config: ValidationConfig {
            in_sample_ratio: 0.5,
            stress_scenarios: vec![],
            parameter_sweeps: vec![],
            walk_forward: None,
            regime_windows: vec![],
            deterministic_seed: 9,
            min_trades_for_score: 1,
        },
    }
}

#[allow(dead_code)]
pub fn sample_event_stream() -> Vec<MarketEvent> {
    let base = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    vec![
        MarketEvent {
            timestamp: base,
            sequence: 1,
            kind: MarketEventKind::Trade {
                price: 100.0,
                quantity: 1.0,
                aggressor: None,
            },
        },
        MarketEvent {
            timestamp: base,
            sequence: 2,
            kind: MarketEventKind::Trade {
                price: 100.5,
                quantity: 1.0,
                aggressor: None,
            },
        },
        MarketEvent {
            timestamp: base + Duration::seconds(1),
            sequence: 3,
            kind: MarketEventKind::Trade {
                price: 101.0,
                quantity: 1.0,
                aggressor: None,
            },
        },
    ]
}
