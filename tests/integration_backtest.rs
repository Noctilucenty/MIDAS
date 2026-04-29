use chrono::{Duration, TimeZone, Utc};
use midas_backtesting_engine::domain::config::{
    BacktestConfig, BacktestRequest, ExecutionConfig, LimitFillAssumption, LiquidationConfig,
    MarketPriceReference, OrderTypeAssumption, PositionSizing, RunContext, StrategyDefinition,
};
use midas_backtesting_engine::domain::types::{Candle, MarketDataSet, StrategyInput};
use midas_backtesting_engine::engine::backtester::BacktestEngine;

fn sample_candles() -> Vec<Candle> {
    let closes = [
        100.0, 101.0, 102.5, 103.5, 102.0, 100.0, 99.0, 100.0, 102.0, 104.0, 106.0, 108.0,
    ];
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| Candle {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                + Duration::hours(index as i64),
            open: *close - 0.5,
            high: *close + 1.0,
            low: *close - 1.0,
            close: *close,
            volume: 1_000.0,
            funding_rate: if index % 4 == 0 { 0.0001 } else { 0.0 },
            spread_bps: Some(3.0),
        })
        .collect()
}

#[test]
fn complete_backtest_run_produces_trades_and_metrics() {
    let request = BacktestRequest {
        context: RunContext {
            symbol: "BTCUSDT-PERP".to_string(),
            venue: Some("sample".to_string()),
            timeframe: "1h".to_string(),
            run_label: Some("integration".to_string()),
        },
        market_data: MarketDataSet::Candles(sample_candles()),
        strategy_input: StrategyInput::Definition(StrategyDefinition::MovingAverageCross {
            fast_window: 2,
            slow_window: 4,
        }),
        backtest_config: BacktestConfig {
            starting_cash: 10_000.0,
            default_leverage: 2.0,
            max_leverage: 3.0,
            position_sizing: PositionSizing::PercentOfEquity { fraction: 0.5 },
            allow_long: true,
            allow_short: true,
        },
        execution_config: ExecutionConfig {
            taker_fee_bps: 5.0,
            maker_fee_bps: 2.0,
            spread_bps: 2.0,
            slippage_bps: 3.0,
            latency_bars: 0,
            latency_events: 0,
            order_timeout_bars: None,
            order_timeout_events: None,
            order_type: OrderTypeAssumption::Market,
            market_price_reference: MarketPriceReference::Mid,
            limit_fill_assumption: LimitFillAssumption::Touch,
            use_candle_spread: true,
            partial_fill_ratio: None,
            liquidation: LiquidationConfig::default(),
        },
    };

    let report = BacktestEngine::run(request, 7).unwrap();
    assert!(report.metrics.trade_count > 0);
    assert!(!report.artifacts.trade_log.is_empty());
    assert_eq!(report.artifacts.equity_curve.len(), 12);
    assert!(report.metrics.ending_equity > 0.0);
}
