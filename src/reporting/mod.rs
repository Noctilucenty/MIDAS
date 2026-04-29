use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::adapters::csv::write_trade_log_csv;
use crate::domain::errors::BacktestError;
use crate::domain::types::{BacktestReport, ExecutionLeg, OrderAuditEntry, ValidationReport};

pub const ARTIFACT_SCHEMA_VERSION: &str = "2.0.0";

pub fn stable_signature<T: Serialize>(value: &T) -> Result<String, BacktestError> {
    let encoded = serde_json::to_vec(value)?;
    let digest = Sha256::digest(encoded);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn write_backtest_report(path: &Path, report: &BacktestReport) -> Result<(), BacktestError> {
    fs::create_dir_all(path)
        .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    write_json(path.join("backtest_report.json").as_path(), report)?;
    write_json(path.join("metrics.json").as_path(), &report.metrics)?;
    write_json(path.join("metrics_summary.json").as_path(), &report.metrics)?;
    write_json(
        path.join("execution_diagnostics.json").as_path(),
        &report.artifacts.execution_diagnostics,
    )?;
    write_json(
        path.join("replay_diagnostics.json").as_path(),
        &report.artifacts.replay_diagnostics,
    )?;
    write_json(path.join("run_manifest.json").as_path(), &report.manifest)?;
    write_json(
        path.join("order_audit_log.json").as_path(),
        &report.artifacts.order_audit_log,
    )?;
    write_json(
        path.join("execution_legs.json").as_path(),
        &report.artifacts.execution_legs,
    )?;
    write_json(
        path.join("consistency_report.json").as_path(),
        &report.artifacts.consistency_report,
    )?;
    write_series_csv(
        path.join("equity_curve.csv").as_path(),
        &report
            .artifacts
            .equity_curve
            .iter()
            .map(|point| {
                vec![
                    point.timestamp.to_rfc3339(),
                    point.equity.to_string(),
                    point.cash.to_string(),
                    point.unrealized_pnl.to_string(),
                    format!("{:?}", point.position_state),
                ]
            })
            .collect::<Vec<_>>(),
        &[
            "timestamp",
            "equity",
            "cash",
            "unrealized_pnl",
            "position_state",
        ],
    )?;
    write_series_csv(
        path.join("drawdown_curve.csv").as_path(),
        &report
            .artifacts
            .drawdown_curve
            .iter()
            .map(|point| {
                vec![
                    point.timestamp.to_rfc3339(),
                    point.equity.to_string(),
                    point.peak_equity.to_string(),
                    point.drawdown.to_string(),
                    point.drawdown_pct.to_string(),
                ]
            })
            .collect::<Vec<_>>(),
        &[
            "timestamp",
            "equity",
            "peak_equity",
            "drawdown",
            "drawdown_pct",
        ],
    )?;
    write_trade_log_csv(
        path.join("trade_log.csv").as_path(),
        &report.artifacts.trade_log,
    )?;
    write_order_audit_csv(
        path.join("order_audit_log.csv").as_path(),
        &report.artifacts.order_audit_log,
    )?;
    write_execution_legs_csv(
        path.join("execution_legs.csv").as_path(),
        &report.artifacts.execution_legs,
    )?;
    Ok(())
}

pub fn write_validation_report(
    path: &Path,
    report: &ValidationReport,
) -> Result<(), BacktestError> {
    fs::create_dir_all(path)
        .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    write_json(path.join("validation_report.json").as_path(), report)?;
    write_json(
        path.join("validation_diagnostics.json").as_path(),
        &report.diagnostics,
    )?;
    write_backtest_report(path.join("base_backtest").as_path(), &report.base_report)?;
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), BacktestError> {
    let file =
        File::create(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer
        .flush()
        .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))
}

fn write_series_csv(
    path: &Path,
    rows: &[Vec<String>],
    headers: &[&str],
) -> Result<(), BacktestError> {
    let file =
        File::create(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(file));
    writer.write_record(headers)?;
    for row in rows {
        writer.write_record(row)?;
    }
    writer
        .flush()
        .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    Ok(())
}

fn write_order_audit_csv(path: &Path, entries: &[OrderAuditEntry]) -> Result<(), BacktestError> {
    let file =
        File::create(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(file));
    writer.write_record([
        "order_id",
        "generated_at",
        "completed_at",
        "status",
        "requested_kind",
        "realized_kind",
        "limit_price",
        "fill_price",
        "quantity",
        "maker",
        "execution_leg_count",
        "position_before",
        "position_after",
        "reason_code",
        "reason",
    ])?;
    for entry in entries {
        writer.write_record([
            entry.order_id.to_string(),
            entry.generated_at.to_rfc3339(),
            entry
                .completed_at
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_default(),
            format!("{:?}", entry.status),
            format!("{:?}", entry.requested_kind),
            entry
                .realized_kind
                .map(|kind| format!("{kind:?}"))
                .unwrap_or_default(),
            entry
                .limit_price
                .map(|value| value.to_string())
                .unwrap_or_default(),
            entry
                .fill_price
                .map(|value| value.to_string())
                .unwrap_or_default(),
            entry
                .quantity
                .map(|value| value.to_string())
                .unwrap_or_default(),
            entry
                .maker
                .map(|value| value.to_string())
                .unwrap_or_default(),
            entry
                .execution_leg_count
                .map(|value| value.to_string())
                .unwrap_or_default(),
            entry
                .position_before
                .map(|value| format!("{value:?}"))
                .unwrap_or_default(),
            entry
                .position_after
                .map(|value| format!("{value:?}"))
                .unwrap_or_default(),
            entry.reason_code.clone(),
            entry.reason.clone(),
        ])?;
    }
    writer
        .flush()
        .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    Ok(())
}

fn write_execution_legs_csv(path: &Path, legs: &[ExecutionLeg]) -> Result<(), BacktestError> {
    let file =
        File::create(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(file));
    writer.write_record([
        "order_id",
        "leg_index",
        "role",
        "generated_at",
        "executed_at",
        "trigger_sequence",
        "requested_kind",
        "realized_kind",
        "side",
        "quantity",
        "reference_price",
        "realized_execution_price",
        "maker",
        "fee_paid",
        "spread_cost",
        "slippage_cost",
        "funding_cost",
        "gross_pnl",
        "net_pnl",
        "fill_delay_steps",
        "fill_delay_seconds",
        "position_before",
        "position_after",
        "liquidation",
        "reason_code",
        "reason",
    ])?;
    for leg in legs {
        writer.write_record([
            leg.order_id.to_string(),
            leg.leg_index.to_string(),
            format!("{:?}", leg.role),
            leg.generated_at.to_rfc3339(),
            leg.executed_at.to_rfc3339(),
            leg.trigger_sequence
                .map(|value| value.to_string())
                .unwrap_or_default(),
            format!("{:?}", leg.requested_kind),
            format!("{:?}", leg.realized_kind),
            format!("{:?}", leg.side),
            leg.quantity.to_string(),
            leg.reference_price.to_string(),
            leg.realized_execution_price.to_string(),
            leg.maker.to_string(),
            leg.fee_paid.to_string(),
            leg.spread_cost.to_string(),
            leg.slippage_cost.to_string(),
            leg.funding_cost.to_string(),
            leg.gross_pnl
                .map(|value| value.to_string())
                .unwrap_or_default(),
            leg.net_pnl
                .map(|value| value.to_string())
                .unwrap_or_default(),
            leg.fill_delay_steps.to_string(),
            leg.fill_delay_seconds.to_string(),
            format!("{:?}", leg.position_before),
            format!("{:?}", leg.position_after),
            leg.liquidation.to_string(),
            leg.reason_code.clone(),
            leg.reason.clone(),
        ])?;
    }
    writer
        .flush()
        .map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    Ok(())
}
