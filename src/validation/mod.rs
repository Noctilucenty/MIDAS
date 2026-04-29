use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::domain::config::{
    BacktestRequest, ParameterSweep, ParameterValue, RegimeWindow, ValidationRequest,
};
use crate::domain::errors::BacktestError;
use crate::domain::types::{
    BacktestReport, MarketDataSet, MarketEventKind, ParameterSweepResult, RegimeReport,
    RobustnessVerdict, SplitReport, StrategyInput, StressTestReport, ValidationDiagnostics,
    ValidationReasonCode, ValidationReport, ValidationScoreBreakdown, ValidationSummary,
    WalkForwardWindowReport,
};
use crate::engine::backtester::BacktestEngine;
use crate::reporting::{stable_signature, ARTIFACT_SCHEMA_VERSION};

pub fn run_validation(request: ValidationRequest) -> Result<ValidationReport, BacktestError> {
    request.validation_config.validate()?;
    let deterministic_seed = request.validation_config.deterministic_seed;
    let base_report = BacktestEngine::run(request.backtest_request.clone(), deterministic_seed)?;
    let (in_sample_request, out_of_sample_request) = split_request(&request)?;
    let in_sample_report = BacktestEngine::run(in_sample_request.clone(), deterministic_seed)?;
    let out_of_sample_report =
        BacktestEngine::run(out_of_sample_request.clone(), deterministic_seed)?;

    let stress_tests = request
        .validation_config
        .stress_scenarios
        .iter()
        .map(|scenario| {
            let mut stressed_request = request.backtest_request.clone();
            stressed_request.execution_config.taker_fee_bps += scenario.fee_bps_delta;
            stressed_request.execution_config.maker_fee_bps += scenario.fee_bps_delta;
            stressed_request.execution_config.spread_bps += scenario.spread_bps_delta;
            stressed_request.execution_config.slippage_bps += scenario.slippage_bps_delta;
            stressed_request.execution_config.latency_bars += scenario.latency_bars_delta;
            stressed_request.execution_config.latency_events += scenario.latency_events_delta;
            scale_funding(
                &mut stressed_request.market_data,
                scenario.funding_multiplier,
            );
            BacktestEngine::run(stressed_request, deterministic_seed).map(|report| {
                let passed = survivability_pass(&report.metrics);
                StressTestReport {
                    scenario_name: scenario.name.clone(),
                    net_pnl_delta_from_base: report.metrics.net_pnl - base_report.metrics.net_pnl,
                    metrics: report.metrics,
                    passed,
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let parameter_sensitivity = parameter_sensitivity_runs(
        &request.backtest_request,
        &request.validation_config.parameter_sweeps,
        deterministic_seed,
        base_report.metrics.net_pnl,
    )?;

    let walk_forward = walk_forward_reports(
        &request.backtest_request,
        request.validation_config.walk_forward.as_ref(),
        deterministic_seed,
    )?;

    let regime_reports = regime_reports(
        &request.backtest_request,
        &request.validation_config.regime_windows,
        deterministic_seed,
    )?;

    let deterministic_reproducible = {
        let run_a = BacktestEngine::run(request.backtest_request.clone(), deterministic_seed)?;
        let run_b = BacktestEngine::run(request.backtest_request.clone(), deterministic_seed)?;
        stable_signature(&run_a)? == stable_signature(&run_b)?
    };

    let summary = summarize_validation(
        &base_report,
        &in_sample_report,
        &out_of_sample_report,
        &stress_tests,
        &parameter_sensitivity,
        &walk_forward,
        deterministic_reproducible,
        request.validation_config.min_trades_for_score,
    );

    let diagnostics = build_diagnostics(
        &in_sample_report,
        &out_of_sample_report,
        &stress_tests,
        &parameter_sensitivity,
        &walk_forward,
        &summary,
    );

    Ok(ValidationReport {
        metadata: crate::domain::config::RunMetadata {
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            artifact_schema_version: ARTIFACT_SCHEMA_VERSION.to_string(),
            run_signature: stable_signature(&request)?,
            deterministic_seed,
        },
        base_report,
        in_sample: SplitReport {
            label: "in_sample".to_string(),
            range_start: range_start(&in_sample_request.market_data)?,
            range_end: range_end(&in_sample_request.market_data)?,
            metrics: in_sample_report.metrics,
        },
        out_of_sample: SplitReport {
            label: "out_of_sample".to_string(),
            range_start: range_start(&out_of_sample_request.market_data)?,
            range_end: range_end(&out_of_sample_request.market_data)?,
            metrics: out_of_sample_report.metrics,
        },
        stress_tests,
        parameter_sensitivity,
        walk_forward,
        regime_reports,
        diagnostics,
        summary,
    })
}

fn split_request(
    request: &ValidationRequest,
) -> Result<(BacktestRequest, BacktestRequest), BacktestError> {
    let len = dataset_len(&request.backtest_request.market_data);
    if len < 4 {
        return Err(BacktestError::InvalidData(
            "validation requires at least 4 observations".to_string(),
        ));
    }
    let split_idx = ((len as f64) * request.validation_config.in_sample_ratio).floor() as usize;
    let split_idx = split_idx.clamp(2, len.saturating_sub(2));
    let mut in_request = request.backtest_request.clone();
    in_request.market_data =
        subset_market_data(&request.backtest_request.market_data, 0, split_idx)?;
    let mut out_request = request.backtest_request.clone();
    out_request.market_data =
        subset_market_data(&request.backtest_request.market_data, split_idx, len)?;
    if let StrategyInput::SignalStream(signals) = &request.backtest_request.strategy_input {
        let in_start = range_start(&in_request.market_data)?;
        let in_end = range_end(&in_request.market_data)?;
        let out_start = range_start(&out_request.market_data)?;
        let out_end = range_end(&out_request.market_data)?;
        in_request.strategy_input = StrategyInput::SignalStream(
            signals
                .iter()
                .filter(|signal| signal.timestamp >= in_start && signal.timestamp <= in_end)
                .cloned()
                .collect(),
        );
        out_request.strategy_input = StrategyInput::SignalStream(
            signals
                .iter()
                .filter(|signal| signal.timestamp >= out_start && signal.timestamp <= out_end)
                .cloned()
                .collect(),
        );
    }
    Ok((in_request, out_request))
}

fn parameter_sensitivity_runs(
    base_request: &BacktestRequest,
    sweeps: &[ParameterSweep],
    deterministic_seed: u64,
    base_net_pnl: f64,
) -> Result<Vec<ParameterSweepResult>, BacktestError> {
    if sweeps.is_empty() {
        return Ok(Vec::new());
    }
    let StrategyInput::Definition(definition) = &base_request.strategy_input else {
        return Ok(Vec::new());
    };
    let combinations = sweep_combinations(sweeps);
    combinations
        .into_iter()
        .map(|combo| {
            let mut tuned_definition = definition.clone();
            for (name, value) in &combo {
                tuned_definition = tuned_definition.apply_parameter(name, value)?;
                tuned_definition.validate()?;
            }
            let mut request = base_request.clone();
            request.strategy_input = StrategyInput::Definition(tuned_definition);
            BacktestEngine::run(request, deterministic_seed).map(|report| {
                let metrics = report.metrics;
                ParameterSweepResult {
                    parameter_values: combo
                        .into_iter()
                        .map(|(key, value)| {
                            let rendered = match value {
                                ParameterValue::Int(value) => value.to_string(),
                                ParameterValue::Float(value) => value.to_string(),
                            };
                            (key, rendered)
                        })
                        .collect::<BTreeMap<_, _>>(),
                    net_pnl_delta_from_base: metrics.net_pnl - base_net_pnl,
                    passed: survivability_pass(&metrics),
                    metrics,
                }
            })
        })
        .collect()
}

fn walk_forward_reports(
    base_request: &BacktestRequest,
    walk_forward: Option<&crate::domain::config::WalkForwardConfig>,
    deterministic_seed: u64,
) -> Result<Vec<WalkForwardWindowReport>, BacktestError> {
    let Some(walk_forward) = walk_forward else {
        return Ok(Vec::new());
    };
    let len = dataset_len(&base_request.market_data);
    let train_len = ((len as f64) * walk_forward.train_ratio).floor() as usize;
    let test_len = ((len as f64) * walk_forward.test_ratio).floor() as usize;
    let step_len = ((len as f64) * walk_forward.step_ratio).floor() as usize;
    if train_len < 2 || test_len < 1 || step_len < 1 || train_len + test_len > len {
        return Ok(Vec::new());
    }
    let mut reports = Vec::new();
    let mut start = 0usize;
    let mut window_index = 0usize;
    while start + train_len + test_len <= len {
        let training = subset_backtest_request(base_request, start, start + train_len)?;
        let testing = subset_backtest_request(
            base_request,
            start + train_len,
            start + train_len + test_len,
        )?;
        let training_report = BacktestEngine::run(training.clone(), deterministic_seed)?;
        let test_report = BacktestEngine::run(testing.clone(), deterministic_seed)?;
        reports.push(WalkForwardWindowReport {
            window_index,
            training_range_start: range_start(&training.market_data)?,
            training_range_end: range_end(&training.market_data)?,
            test_range_start: range_start(&testing.market_data)?,
            test_range_end: range_end(&testing.market_data)?,
            training_metrics: training_report.metrics,
            test_metrics: test_report.metrics.clone(),
            passed: survivability_pass(&test_report.metrics),
        });
        window_index += 1;
        if walk_forward
            .max_windows
            .map(|max_windows| window_index >= max_windows)
            .unwrap_or(false)
        {
            break;
        }
        start += step_len;
    }
    Ok(reports)
}

fn regime_reports(
    base_request: &BacktestRequest,
    regimes: &[RegimeWindow],
    deterministic_seed: u64,
) -> Result<Vec<RegimeReport>, BacktestError> {
    regimes
        .iter()
        .map(|regime| {
            let filtered =
                filter_market_data_by_time(&base_request.market_data, regime.start, regime.end)?;
            let mut request = base_request.clone();
            request.market_data = filtered;
            if let StrategyInput::SignalStream(signals) = &base_request.strategy_input {
                request.strategy_input = StrategyInput::SignalStream(
                    signals
                        .iter()
                        .filter(|signal| {
                            signal.timestamp >= regime.start && signal.timestamp <= regime.end
                        })
                        .cloned()
                        .collect(),
                );
            }
            BacktestEngine::run(request, deterministic_seed).map(|report| RegimeReport {
                regime: regime.clone(),
                metrics: report.metrics,
            })
        })
        .collect()
}

fn summarize_validation(
    base: &BacktestReport,
    in_sample: &BacktestReport,
    out_of_sample: &BacktestReport,
    stress_tests: &[StressTestReport],
    sensitivity: &[ParameterSweepResult],
    walk_forward: &[WalkForwardWindowReport],
    deterministic_reproducible: bool,
    min_trades_for_score: usize,
) -> ValidationSummary {
    let stress_pass_rate = pass_rate(stress_tests.iter().map(|test| test.passed));
    let sensitivity_pass_rate = pass_rate(sensitivity.iter().map(|test| test.passed));
    let walk_forward_pass_rate = pass_rate(walk_forward.iter().map(|window| window.passed));
    let trade_sufficiency = if out_of_sample.metrics.trade_count >= min_trades_for_score {
        5.0
    } else {
        (out_of_sample.metrics.trade_count as f64 / min_trades_for_score as f64) * 5.0
    };
    let degradation_ratio = if in_sample.metrics.ending_equity.abs() < f64::EPSILON {
        1.0
    } else {
        out_of_sample.metrics.ending_equity / in_sample.metrics.ending_equity
    }
    .clamp(0.0, 1.2);

    let profitability = if out_of_sample.metrics.net_pnl > 0.0 {
        20.0
    } else {
        0.0
    };
    let drawdown_control = (1.0 - out_of_sample.metrics.max_drawdown_pct.min(0.5) / 0.5) * 20.0;
    let stability = ((degradation_ratio.min(1.0) + walk_forward_pass_rate) / 2.0) * 20.0;
    let sensitivity_score = sensitivity_pass_rate * 10.0;
    let stress_score = stress_pass_rate * 20.0;
    let determinism_score = if deterministic_reproducible { 5.0 } else { 0.0 };
    let total = profitability
        + drawdown_control
        + stability
        + sensitivity_score
        + stress_score
        + determinism_score
        + trade_sufficiency;
    let breakdown = ValidationScoreBreakdown {
        profitability,
        drawdown_control,
        stability,
        sensitivity: sensitivity_score,
        stress_survivability: stress_score,
        determinism: determinism_score,
        trade_sufficiency,
        total,
        max_total: 100.0,
    };
    let verdict = if deterministic_reproducible
        && survivability_pass(&out_of_sample.metrics)
        && stress_pass_rate >= 0.5
        && total >= 75.0
    {
        RobustnessVerdict::Passes
    } else if total >= 60.0 {
        RobustnessVerdict::Borderline
    } else if total >= 40.0 {
        RobustnessVerdict::Fragile
    } else {
        RobustnessVerdict::Fails
    };
    let reason_codes = derive_reason_codes(
        out_of_sample,
        stress_tests,
        sensitivity,
        walk_forward,
        deterministic_reproducible,
        min_trades_for_score,
    );
    let primary_reason = reason_codes.first().copied();
    let passed = matches!(verdict, RobustnessVerdict::Passes);
    let summary = format!(
        "Base net pnl {:.2}, OOS net pnl {:.2}, OOS max drawdown {:.2}%, stress pass {:.0}%, sensitivity pass {:.0}%, walk-forward pass {:.0}%, verdict {:?}.",
        base.metrics.net_pnl,
        out_of_sample.metrics.net_pnl,
        out_of_sample.metrics.max_drawdown_pct * 100.0,
        stress_pass_rate * 100.0,
        sensitivity_pass_rate * 100.0,
        walk_forward_pass_rate * 100.0,
        verdict
    );
    ValidationSummary {
        score: total,
        passed,
        verdict,
        deterministic_reproducible,
        stress_pass_rate,
        sensitivity_pass_rate,
        walk_forward_pass_rate,
        reason_codes,
        primary_reason,
        breakdown,
        summary,
    }
}

fn build_diagnostics(
    in_sample: &BacktestReport,
    out_of_sample: &BacktestReport,
    stress_tests: &[StressTestReport],
    parameter_sensitivity: &[ParameterSweepResult],
    _walk_forward: &[WalkForwardWindowReport],
    summary: &ValidationSummary,
) -> ValidationDiagnostics {
    let degradation_ratio = if in_sample.metrics.ending_equity.abs() < f64::EPSILON {
        1.0
    } else {
        out_of_sample.metrics.ending_equity / in_sample.metrics.ending_equity
    };
    let mut score_explanation = Vec::new();
    score_explanation.push(format!(
        "Profitability component: {:.2}/20",
        summary.breakdown.profitability
    ));
    score_explanation.push(format!(
        "Drawdown control component: {:.2}/20",
        summary.breakdown.drawdown_control
    ));
    score_explanation.push(format!(
        "Stability component: {:.2}/20",
        summary.breakdown.stability
    ));
    score_explanation.push(format!(
        "Stress survivability component: {:.2}/20",
        summary.breakdown.stress_survivability
    ));
    let verdict_explanation = summary
        .reason_codes
        .iter()
        .map(|code| reason_code_message(*code))
        .collect();
    ValidationDiagnostics {
        degradation_ratio,
        walk_forward_pass_rate: summary.walk_forward_pass_rate,
        worst_stress_net_pnl: stress_tests
            .iter()
            .map(|test| test.metrics.net_pnl)
            .min_by(|left, right| left.total_cmp(right)),
        best_parameter_net_pnl: parameter_sensitivity
            .iter()
            .map(|result| result.metrics.net_pnl)
            .max_by(|left, right| left.total_cmp(right)),
        worst_parameter_net_pnl: parameter_sensitivity
            .iter()
            .map(|result| result.metrics.net_pnl)
            .min_by(|left, right| left.total_cmp(right)),
        score_explanation,
        verdict_explanation,
    }
}

fn survivability_pass(metrics: &crate::domain::types::MetricsReport) -> bool {
    metrics.net_pnl > 0.0 && metrics.max_drawdown_pct < 0.35
}

fn pass_rate(items: impl Iterator<Item = bool>) -> f64 {
    let items = items.collect::<Vec<_>>();
    if items.is_empty() {
        0.0
    } else {
        items.iter().filter(|passed| **passed).count() as f64 / items.len() as f64
    }
}

fn derive_reason_codes(
    out_of_sample: &BacktestReport,
    stress_tests: &[StressTestReport],
    sensitivity: &[ParameterSweepResult],
    walk_forward: &[WalkForwardWindowReport],
    deterministic_reproducible: bool,
    min_trades_for_score: usize,
) -> Vec<ValidationReasonCode> {
    let mut codes = BTreeSet::new();
    if out_of_sample.metrics.net_pnl <= 0.0 {
        codes.insert(ValidationReasonCode::NegativeOutOfSamplePnl);
    }
    if out_of_sample.metrics.max_drawdown_pct >= 0.35 {
        codes.insert(ValidationReasonCode::ExcessiveOutOfSampleDrawdown);
    }
    if out_of_sample.metrics.trade_count < min_trades_for_score {
        codes.insert(ValidationReasonCode::InsufficientTrades);
    }
    if !deterministic_reproducible {
        codes.insert(ValidationReasonCode::NonDeterministicResults);
    }
    if stress_tests.is_empty() || sensitivity.is_empty() || walk_forward.is_empty() {
        codes.insert(ValidationReasonCode::InsufficientValidationCoverage);
    }
    if !stress_tests.is_empty() && pass_rate(stress_tests.iter().map(|test| test.passed)) < 0.5 {
        codes.insert(ValidationReasonCode::StressFailures);
    }
    if !sensitivity.is_empty() && pass_rate(sensitivity.iter().map(|test| test.passed)) < 0.5 {
        codes.insert(ValidationReasonCode::ParameterInstability);
    }
    if !walk_forward.is_empty() && pass_rate(walk_forward.iter().map(|window| window.passed)) < 0.5
    {
        codes.insert(ValidationReasonCode::WalkForwardInstability);
    }
    codes.into_iter().collect()
}

fn reason_code_message(code: ValidationReasonCode) -> String {
    match code {
        ValidationReasonCode::NegativeOutOfSamplePnl => {
            "Out-of-sample profitability was not positive".to_string()
        }
        ValidationReasonCode::ExcessiveOutOfSampleDrawdown => {
            "Out-of-sample drawdown exceeded the survivability threshold".to_string()
        }
        ValidationReasonCode::StressFailures => {
            "Stress scenarios degraded beyond acceptable survivability".to_string()
        }
        ValidationReasonCode::ParameterInstability => {
            "Parameter sensitivity indicates unstable robustness".to_string()
        }
        ValidationReasonCode::WalkForwardInstability => {
            "Walk-forward windows did not show stable survivability".to_string()
        }
        ValidationReasonCode::NonDeterministicResults => {
            "Repeated deterministic runs did not reconcile".to_string()
        }
        ValidationReasonCode::InsufficientTrades => {
            "Out-of-sample evidence contains too few trades".to_string()
        }
        ValidationReasonCode::InsufficientValidationCoverage => {
            "One or more optional robustness checks were omitted, reducing evidence quality"
                .to_string()
        }
    }
}

fn scale_funding(dataset: &mut MarketDataSet, multiplier: f64) {
    match dataset {
        MarketDataSet::Candles(candles) => {
            for candle in candles {
                candle.funding_rate *= multiplier;
            }
        }
        MarketDataSet::Events(events) => {
            for event in events {
                if let MarketEventKind::Funding { rate } = &mut event.kind {
                    *rate *= multiplier;
                }
                if let MarketEventKind::Candle { candle } = &mut event.kind {
                    candle.funding_rate *= multiplier;
                }
            }
        }
    }
}

fn sweep_combinations(sweeps: &[ParameterSweep]) -> Vec<BTreeMap<String, ParameterValue>> {
    fn recurse(
        idx: usize,
        sweeps: &[ParameterSweep],
        current: &mut BTreeMap<String, ParameterValue>,
        results: &mut Vec<BTreeMap<String, ParameterValue>>,
    ) {
        if idx == sweeps.len() {
            results.push(current.clone());
            return;
        }
        let sweep = &sweeps[idx];
        for value in &sweep.values {
            current.insert(sweep.name.clone(), value.clone());
            recurse(idx + 1, sweeps, current, results);
        }
        current.remove(&sweep.name);
    }

    let mut results = Vec::new();
    let mut current = BTreeMap::new();
    recurse(0, sweeps, &mut current, &mut results);
    results
}

fn subset_backtest_request(
    base_request: &BacktestRequest,
    start: usize,
    end: usize,
) -> Result<BacktestRequest, BacktestError> {
    let mut request = base_request.clone();
    request.market_data = subset_market_data(&base_request.market_data, start, end)?;
    if let StrategyInput::SignalStream(signals) = &base_request.strategy_input {
        let start_ts = range_start(&request.market_data)?;
        let end_ts = range_end(&request.market_data)?;
        request.strategy_input = StrategyInput::SignalStream(
            signals
                .iter()
                .filter(|signal| signal.timestamp >= start_ts && signal.timestamp <= end_ts)
                .cloned()
                .collect(),
        );
    }
    Ok(request)
}

fn subset_market_data(
    dataset: &MarketDataSet,
    start: usize,
    end: usize,
) -> Result<MarketDataSet, BacktestError> {
    match dataset {
        MarketDataSet::Candles(candles) => Ok(MarketDataSet::Candles(candles[start..end].to_vec())),
        MarketDataSet::Events(events) => Ok(MarketDataSet::Events(events[start..end].to_vec())),
    }
}

fn filter_market_data_by_time(
    dataset: &MarketDataSet,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<MarketDataSet, BacktestError> {
    let filtered = match dataset {
        MarketDataSet::Candles(candles) => MarketDataSet::Candles(
            candles
                .iter()
                .filter(|candle| candle.timestamp >= start && candle.timestamp <= end)
                .cloned()
                .collect(),
        ),
        MarketDataSet::Events(events) => MarketDataSet::Events(
            events
                .iter()
                .filter(|event| event.timestamp >= start && event.timestamp <= end)
                .cloned()
                .collect(),
        ),
    };
    if dataset_len(&filtered) == 0 {
        return Err(BacktestError::InvalidData(
            "regime window does not overlap any data".to_string(),
        ));
    }
    Ok(filtered)
}

fn dataset_len(dataset: &MarketDataSet) -> usize {
    match dataset {
        MarketDataSet::Candles(candles) => candles.len(),
        MarketDataSet::Events(events) => events.len(),
    }
}

fn range_start(dataset: &MarketDataSet) -> Result<chrono::DateTime<chrono::Utc>, BacktestError> {
    match dataset {
        MarketDataSet::Candles(candles) => candles.first().map(|candle| candle.timestamp),
        MarketDataSet::Events(events) => events.first().map(|event| event.timestamp),
    }
    .ok_or_else(|| BacktestError::InvalidData("dataset is empty".to_string()))
}

fn range_end(dataset: &MarketDataSet) -> Result<chrono::DateTime<chrono::Utc>, BacktestError> {
    match dataset {
        MarketDataSet::Candles(candles) => candles.last().map(|candle| candle.timestamp),
        MarketDataSet::Events(events) => events.last().map(|event| event.timestamp),
    }
    .ok_or_else(|| BacktestError::InvalidData("dataset is empty".to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::{run_validation, split_request};
    use crate::domain::config::{
        BacktestConfig, BacktestRequest, ExecutionConfig, LimitFillAssumption, LiquidationConfig,
        MarketPriceReference, OrderTypeAssumption, ParameterSweep, ParameterValue, PositionSizing,
        RegimeWindow, RunContext, StrategyDefinition, ValidationConfig, ValidationRequest,
        WalkForwardConfig,
    };
    use crate::domain::types::{Candle, MarketDataSet, StrategyInput};

    fn candles() -> Vec<Candle> {
        (0..12)
            .map(|index| Candle {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                    + Duration::hours(index as i64),
                open: 100.0 + index as f64,
                high: 101.0 + index as f64,
                low: 99.0 + index as f64,
                close: 100.5 + index as f64,
                volume: 10.0,
                funding_rate: 0.0,
                spread_bps: None,
            })
            .collect()
    }

    fn request() -> ValidationRequest {
        ValidationRequest {
            backtest_request: BacktestRequest {
                context: RunContext {
                    symbol: "BTC-PERP".to_string(),
                    venue: Some("test".to_string()),
                    timeframe: "1h".to_string(),
                    run_label: Some("validation".to_string()),
                },
                market_data: MarketDataSet::Candles(candles()),
                strategy_input: StrategyInput::Definition(StrategyDefinition::MovingAverageCross {
                    fast_window: 2,
                    slow_window: 4,
                }),
                backtest_config: BacktestConfig {
                    starting_cash: 1_000.0,
                    default_leverage: 1.5,
                    max_leverage: 2.0,
                    position_sizing: PositionSizing::FixedNotional { notional: 500.0 },
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
                    use_candle_spread: false,
                    partial_fill_ratio: None,
                    liquidation: LiquidationConfig::default(),
                },
            },
            validation_config: ValidationConfig {
                in_sample_ratio: 0.5,
                stress_scenarios: vec![],
                parameter_sweeps: vec![ParameterSweep {
                    name: "fast_window".to_string(),
                    values: vec![ParameterValue::Int(2), ParameterValue::Int(3)],
                }],
                walk_forward: Some(WalkForwardConfig {
                    train_ratio: 0.5,
                    test_ratio: 0.25,
                    step_ratio: 0.25,
                    max_windows: Some(2),
                }),
                regime_windows: vec![RegimeWindow {
                    name: "early".to_string(),
                    start: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                    end: Utc.with_ymd_and_hms(2024, 1, 1, 5, 0, 0).unwrap(),
                }],
                deterministic_seed: 42,
                min_trades_for_score: 1,
            },
        }
    }

    #[test]
    fn validation_split_respects_ratio() {
        let request = request();
        let (in_req, out_req) = split_request(&request).unwrap();
        assert_eq!(in_req.market_data.candles().len(), 6);
        assert_eq!(out_req.market_data.candles().len(), 6);
    }

    #[test]
    fn validation_is_deterministic() {
        let report = run_validation(request()).unwrap();
        assert!(report.summary.deterministic_reproducible);
        assert_eq!(report.walk_forward.len(), 2);
        assert_eq!(report.regime_reports.len(), 1);
    }

    #[test]
    fn missing_validation_checks_do_not_count_as_passes() {
        let mut request = request();
        request.validation_config.stress_scenarios.clear();
        request.validation_config.parameter_sweeps.clear();
        request.validation_config.walk_forward = None;
        request.validation_config.regime_windows.clear();

        let report = run_validation(request).unwrap();

        assert_eq!(report.summary.stress_pass_rate, 0.0);
        assert_eq!(report.summary.sensitivity_pass_rate, 0.0);
        assert_eq!(report.summary.walk_forward_pass_rate, 0.0);
        assert_eq!(report.summary.breakdown.stress_survivability, 0.0);
        assert_eq!(report.summary.breakdown.sensitivity, 0.0);
    }
}
