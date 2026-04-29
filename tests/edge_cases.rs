mod common;

use chrono::{TimeZone, Utc};

use common::{
    sample_event_stream, sample_execution_config, signal_stream_request, validation_request,
};
use midas_backtesting_engine::domain::config::{
    BacktestRequest, MarketPriceReference, OrderTypeAssumption, RunContext,
};
use midas_backtesting_engine::domain::errors::BacktestError;
use midas_backtesting_engine::domain::types::{
    ExecutionLegRole, MarketDataSet, MarketEvent, MarketEventKind, PositionState, Signal,
    SignalAction, StrategyInput, ValidationReasonCode,
};
use midas_backtesting_engine::engine::backtester::BacktestEngine;
use midas_backtesting_engine::validation::run_validation;

#[test]
fn empty_signal_stream_produces_zero_trades() {
    let report = BacktestEngine::run(
        signal_stream_request(vec![], sample_execution_config(OrderTypeAssumption::Market)),
        1,
    )
    .unwrap();
    assert_eq!(report.metrics.trade_count, 0);
    assert!(report.artifacts.trade_log.is_empty());
    assert!(report
        .artifacts
        .consistency_report
        .checks
        .iter()
        .all(|check| check.passed));
}

#[test]
fn single_trade_reconciles_through_end_of_backtest_flatten() {
    let report = BacktestEngine::run(
        signal_stream_request(
            vec![Signal {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                action: SignalAction::GoLong,
                leverage_override: Some(1.0),
                limit_price: None,
                note: Some("single_trade".to_string()),
            }],
            sample_execution_config(OrderTypeAssumption::Market),
        ),
        2,
    )
    .unwrap();
    assert_eq!(report.metrics.trade_count, 1);
    assert_eq!(report.artifacts.execution_legs.len(), 2);
    assert_eq!(
        report.artifacts.execution_legs[0].role,
        ExecutionLegRole::Open
    );
}

#[test]
fn flip_order_creates_separate_close_and_open_audit_legs() {
    let report = BacktestEngine::run(
        signal_stream_request(
            vec![
                Signal {
                    timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                    action: SignalAction::GoLong,
                    leverage_override: Some(1.0),
                    limit_price: None,
                    note: Some("go_long".to_string()),
                },
                Signal {
                    timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap(),
                    action: SignalAction::GoShort,
                    leverage_override: Some(1.0),
                    limit_price: None,
                    note: Some("flip_short".to_string()),
                },
            ],
            sample_execution_config(OrderTypeAssumption::Market),
        ),
        3,
    )
    .unwrap();

    let flip_entry = report
        .artifacts
        .order_audit_log
        .iter()
        .find(|entry| entry.order_id == 2 && entry.execution_leg_count == Some(2))
        .unwrap();
    assert!(flip_entry.quantity.is_none());
    let flip_legs = report
        .artifacts
        .execution_legs
        .iter()
        .filter(|leg| leg.order_id == 2)
        .collect::<Vec<_>>();
    assert_eq!(flip_legs.len(), 2);
    assert_eq!(flip_legs[0].position_after, PositionState::Flat);
    assert_eq!(flip_legs[1].position_after, PositionState::Short);
}

#[test]
fn limit_orders_expire_without_filling_after_timeout() {
    let mut execution = sample_execution_config(OrderTypeAssumption::Limit);
    execution.order_timeout_bars = Some(1);
    let report = BacktestEngine::run(
        signal_stream_request(
            vec![Signal {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                action: SignalAction::GoLong,
                leverage_override: Some(1.0),
                limit_price: Some(50.0),
                note: Some("never_fill".to_string()),
            }],
            execution,
        ),
        4,
    )
    .unwrap();
    assert_eq!(report.artifacts.fills.len(), 0);
    assert_eq!(report.artifacts.execution_diagnostics.expired_orders, 1);
}

#[test]
fn duplicate_event_sequences_are_rejected() {
    let request = BacktestRequest {
        context: RunContext {
            symbol: "BTC-PERP".to_string(),
            venue: Some("event".to_string()),
            timeframe: "tick".to_string(),
            run_label: Some("duplicate".to_string()),
        },
        market_data: MarketDataSet::Events(vec![
            MarketEvent {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                sequence: 1,
                kind: MarketEventKind::Trade {
                    price: 100.0,
                    quantity: 1.0,
                    aggressor: None,
                },
            },
            MarketEvent {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                sequence: 1,
                kind: MarketEventKind::Trade {
                    price: 100.5,
                    quantity: 1.0,
                    aggressor: None,
                },
            },
        ]),
        strategy_input: StrategyInput::SignalStream(vec![Signal {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            action: SignalAction::GoLong,
            leverage_override: Some(1.0),
            limit_price: None,
            note: Some("event".to_string()),
        }]),
        backtest_config: signal_stream_request(
            vec![],
            sample_execution_config(OrderTypeAssumption::Market),
        )
        .backtest_config,
        execution_config: sample_execution_config(OrderTypeAssumption::Market),
    };

    let error = BacktestEngine::run(request, 5).unwrap_err();
    assert!(matches!(error, BacktestError::InvalidData(_)));
}

#[test]
fn validation_marks_missing_optional_coverage_explicitly() {
    let report = run_validation(validation_request()).unwrap();
    assert!(report
        .summary
        .reason_codes
        .contains(&ValidationReasonCode::InsufficientValidationCoverage));
}

#[test]
fn event_stream_with_monotonic_sequences_runs() {
    let request = BacktestRequest {
        context: RunContext {
            symbol: "BTC-PERP".to_string(),
            venue: Some("event".to_string()),
            timeframe: "tick".to_string(),
            run_label: Some("event_ok".to_string()),
        },
        market_data: MarketDataSet::Events(sample_event_stream()),
        strategy_input: StrategyInput::SignalStream(vec![Signal {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            action: SignalAction::GoLong,
            leverage_override: Some(1.0),
            limit_price: None,
            note: Some("event".to_string()),
        }]),
        backtest_config: signal_stream_request(
            vec![],
            sample_execution_config(OrderTypeAssumption::Market),
        )
        .backtest_config,
        execution_config: {
            let mut config = sample_execution_config(OrderTypeAssumption::Market);
            config.market_price_reference = MarketPriceReference::LastTrade;
            config
        },
    };
    let report = BacktestEngine::run(request, 6).unwrap();
    assert_eq!(report.artifacts.replay_diagnostics.processed_events, 3);
}
