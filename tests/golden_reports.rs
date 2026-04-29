mod common;

use std::fs;
use std::path::Path;

use chrono::TimeZone;
use common::{signal_stream_request, validation_request};
use midas_backtesting_engine::domain::config::OrderTypeAssumption;
use midas_backtesting_engine::domain::types::{Signal, SignalAction};
use midas_backtesting_engine::engine::backtester::BacktestEngine;
use midas_backtesting_engine::validation::run_validation;

#[test]
fn backtest_artifact_contract_matches_golden_files() {
    let report = BacktestEngine::run(
        signal_stream_request(
            vec![
                Signal {
                    timestamp: chrono::Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                    action: SignalAction::GoLong,
                    leverage_override: Some(1.0),
                    limit_price: None,
                    note: Some("golden_long".to_string()),
                },
                Signal {
                    timestamp: chrono::Utc.with_ymd_and_hms(2024, 1, 1, 2, 0, 0).unwrap(),
                    action: SignalAction::GoShort,
                    leverage_override: Some(1.0),
                    limit_price: None,
                    note: Some("golden_flip".to_string()),
                },
            ],
            common::sample_execution_config(OrderTypeAssumption::Market),
        ),
        7,
    )
    .unwrap();

    assert_fixture(
        "tests/golden/backtest_report.json",
        &serde_json::to_string_pretty(&report).unwrap(),
    );
    assert_fixture(
        "tests/golden/run_manifest.json",
        &serde_json::to_string_pretty(&report.manifest).unwrap(),
    );
    assert_fixture(
        "tests/golden/execution_diagnostics.json",
        &serde_json::to_string_pretty(&report.artifacts.execution_diagnostics).unwrap(),
    );
    assert_fixture(
        "tests/golden/replay_diagnostics.json",
        &serde_json::to_string_pretty(&report.artifacts.replay_diagnostics).unwrap(),
    );
    assert_fixture(
        "tests/golden/order_audit_log.json",
        &serde_json::to_string_pretty(&report.artifacts.order_audit_log).unwrap(),
    );
    assert_fixture(
        "tests/golden/execution_legs.json",
        &serde_json::to_string_pretty(&report.artifacts.execution_legs).unwrap(),
    );
    assert_fixture(
        "tests/golden/consistency_report.json",
        &serde_json::to_string_pretty(&report.artifacts.consistency_report).unwrap(),
    );
}

#[test]
fn validation_artifact_contract_matches_golden_files() {
    let report = run_validation(validation_request()).unwrap();
    assert_fixture(
        "tests/golden/validation_report.json",
        &serde_json::to_string_pretty(&report).unwrap(),
    );
}

fn assert_fixture(path: &str, actual: &str) {
    if std::env::var("UPDATE_GOLDENS").ok().as_deref() == Some("1") {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(Path::new(path), actual).unwrap();
    }
    let expected = fs::read_to_string(Path::new(path)).unwrap();
    assert_eq!(
        expected.trim(),
        actual.trim(),
        "fixture mismatch for {path}"
    );
}
