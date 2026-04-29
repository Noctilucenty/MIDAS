use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::domain::config::{BacktestRequest, OrderTypeAssumption, RunMetadata};
use crate::domain::errors::BacktestError;
use crate::domain::types::{
    BacktestArtifacts, BacktestReport, Candle, DrawdownPoint, EquityPoint, MarketDataSet,
    MarketEvent, MarketEventKind, OrderIntent, OrderKind, PositionState, ReplayDiagnostics,
    RunManifest, Signal, SignalAction, StrategyInput,
};
use crate::engine::invariants::check_backtest_consistency;
use crate::engine::state::BacktestState;
use crate::engine::strategy::build_strategy;
use crate::execution::model::{ExecutionModel, ExecutionVenueSnapshot};
use crate::metrics::compute_metrics;
use crate::reporting::{stable_signature, ARTIFACT_SCHEMA_VERSION};

pub struct BacktestEngine;

#[derive(Debug)]
struct ReplayArtifacts {
    state: BacktestState,
    equity_curve: Vec<EquityPoint>,
    drawdown_curve: Vec<DrawdownPoint>,
    assumptions: BTreeMap<String, String>,
}

impl BacktestEngine {
    pub fn run(
        request: BacktestRequest,
        deterministic_seed: u64,
    ) -> Result<BacktestReport, BacktestError> {
        request.backtest_config.validate()?;
        request.execution_config.validate()?;
        if let StrategyInput::Definition(definition) = &request.strategy_input {
            definition.validate()?;
        }

        let strategy_input = request.strategy_input.clone();
        let replay = match &request.market_data {
            MarketDataSet::Candles(candles) => Self::run_candle_replay(&request, candles.clone())?,
            MarketDataSet::Events(events) => Self::run_event_replay(&request, events.clone())?,
        };

        let mut state = replay.state;
        state.finalize_diagnostics();
        let metrics = compute_metrics(
            request.backtest_config.starting_cash,
            &replay.equity_curve,
            &replay.drawdown_curve,
            &state.trades,
            state.fee_impact,
            state.funding_impact,
            state.slippage_impact,
            state.spread_impact,
            state.exposed_bars,
        );
        let consistency_report = check_backtest_consistency(
            request.backtest_config.starting_cash,
            &state,
            &replay.equity_curve,
            &replay.drawdown_curve,
            &metrics,
        )?;
        let request_signature = stable_signature(&request)?;
        let metadata = RunMetadata {
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            artifact_schema_version: ARTIFACT_SCHEMA_VERSION.to_string(),
            run_signature: request_signature.clone(),
            deterministic_seed,
        };
        let manifest = Self::build_manifest(&request, &request_signature, &replay.assumptions);
        Ok(BacktestReport {
            metadata,
            manifest,
            context: request.context,
            backtest_config: request.backtest_config,
            execution_config: request.execution_config,
            strategy_input,
            metrics,
            artifacts: BacktestArtifacts {
                equity_curve: replay.equity_curve,
                drawdown_curve: replay.drawdown_curve,
                trade_log: state.trades,
                fills: state.fills,
                order_audit_log: state.order_audit_log,
                execution_legs: state.execution_legs,
                execution_diagnostics: state.execution_diagnostics,
                replay_diagnostics: state.replay_diagnostics,
                consistency_report,
                assumptions: replay.assumptions,
            },
        })
    }

    fn run_candle_replay(
        request: &BacktestRequest,
        candles: Vec<Candle>,
    ) -> Result<ReplayArtifacts, BacktestError> {
        Self::validate_candles(&candles)?;
        let execution_model = ExecutionModel;
        let mut state = BacktestState {
            cash: request.backtest_config.starting_cash,
            replay_diagnostics: ReplayDiagnostics {
                mode: "candles".to_string(),
                processed_candles: candles.len(),
                processed_steps: candles.len(),
                ..ReplayDiagnostics::default()
            },
            ..BacktestState::default()
        };
        let mut pending_order: Option<OrderIntent> = None;
        let mut equity_curve = Vec::with_capacity(candles.len());
        let mut drawdown_curve = Vec::with_capacity(candles.len());
        let mut peak_equity = request.backtest_config.starting_cash;
        let signals = sorted_signal_stream(&request.strategy_input);
        let mut signal_cursor = 0usize;
        let mut rule_strategy = match &request.strategy_input {
            StrategyInput::Definition(definition) => Some(build_strategy(definition)?),
            StrategyInput::SignalStream(_) => None,
        };

        for (index, candle) in candles.iter().enumerate() {
            ExecutionModel::apply_candle_funding(&mut state, candle);
            if let Some(intent) = pending_order.take() {
                pending_order = execution_model.process_candle_pending_order(
                    &mut state,
                    intent,
                    candle,
                    index,
                    &request.backtest_config,
                    &request.execution_config,
                );
            }
            ExecutionModel::maybe_liquidate_candle(&mut state, candle, &request.execution_config);
            Self::record_equity_point(
                &mut state,
                candle.timestamp,
                candle.close,
                &mut equity_curve,
                &mut drawdown_curve,
                &mut peak_equity,
            );

            while signal_cursor < signals.len()
                && signals[signal_cursor].timestamp <= candle.timestamp
            {
                Self::schedule_signal(
                    &mut state,
                    &mut pending_order,
                    signals[signal_cursor].clone(),
                    index,
                    request,
                    candle.timestamp,
                    candles.len(),
                    false,
                );
                signal_cursor += 1;
            }

            if let Some(strategy) = rule_strategy.as_mut() {
                let position_state = state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat);
                if let Some(signal) = strategy.on_candle(index, &candles, position_state) {
                    if !matches!(signal.action, SignalAction::Hold) {
                        Self::schedule_signal(
                            &mut state,
                            &mut pending_order,
                            signal,
                            index,
                            request,
                            candle.timestamp,
                            candles.len(),
                            false,
                        );
                    }
                }
            }
        }

        if let Some(intent) = pending_order.take() {
            let completed_at = candles.last().map(|candle| candle.timestamp).unwrap();
            ExecutionModel::cancel_order(
                &mut state,
                &intent,
                completed_at,
                "replay_ended_before_fill",
            );
        }
        if let Some(last_candle) = candles.last() {
            execution_model.force_flatten_candle(
                &mut state,
                last_candle,
                candles.len().saturating_sub(1),
                &request.backtest_config,
                &request.execution_config,
            );
            if state.position.is_none() {
                if let Some(last_point) = equity_curve.last_mut() {
                    last_point.cash = state.cash;
                    last_point.unrealized_pnl = 0.0;
                    last_point.equity = state.cash;
                    last_point.position_state = PositionState::Flat;
                }
                if let Some(last_drawdown) = drawdown_curve.last_mut() {
                    last_drawdown.equity = state.cash;
                    last_drawdown.peak_equity = last_drawdown.peak_equity.max(state.cash);
                    last_drawdown.drawdown = last_drawdown.peak_equity - state.cash;
                    last_drawdown.drawdown_pct = if last_drawdown.peak_equity > 0.0 {
                        last_drawdown.drawdown / last_drawdown.peak_equity
                    } else {
                        0.0
                    };
                }
            }
        }

        Ok(ReplayArtifacts {
            state,
            equity_curve,
            drawdown_curve,
            assumptions: Self::assumption_log(request, "candles"),
        })
    }

    fn run_event_replay(
        request: &BacktestRequest,
        events: Vec<MarketEvent>,
    ) -> Result<ReplayArtifacts, BacktestError> {
        let (events, reordered_events) = Self::prepare_events(&events)?;
        if events.len() < 3 {
            return Err(BacktestError::InvalidData(
                "at least 3 events are required to run an event-driven backtest".to_string(),
            ));
        }
        if matches!(request.strategy_input, StrategyInput::Definition(_))
            && !events
                .iter()
                .any(|event| matches!(event.kind, MarketEventKind::Candle { .. }))
        {
            return Err(BacktestError::Unsupported(
                "rule-based strategies require candle events inside MarketDataSet::Events"
                    .to_string(),
            ));
        }
        let execution_model = ExecutionModel;
        let mut state = BacktestState {
            cash: request.backtest_config.starting_cash,
            replay_diagnostics: ReplayDiagnostics {
                mode: "events".to_string(),
                processed_events: events.len(),
                processed_steps: events.len(),
                reordered_events,
                ..ReplayDiagnostics::default()
            },
            ..BacktestState::default()
        };
        let mut pending_order: Option<OrderIntent> = None;
        let mut equity_curve = Vec::with_capacity(events.len());
        let mut drawdown_curve = Vec::with_capacity(events.len());
        let mut peak_equity = request.backtest_config.starting_cash;
        let signals = sorted_signal_stream(&request.strategy_input);
        let mut signal_cursor = 0usize;
        let mut rule_strategy = match &request.strategy_input {
            StrategyInput::Definition(definition) => Some(build_strategy(definition)?),
            StrategyInput::SignalStream(_) => None,
        };
        let mut venue_snapshot = ExecutionVenueSnapshot::default();
        let mut candle_events = Vec::new();

        for (index, event) in events.iter().enumerate() {
            venue_snapshot.apply_event(event);
            state.replay_diagnostics.last_sequence = Some(event.sequence);
            match &event.kind {
                MarketEventKind::Quote { .. } => state.replay_diagnostics.quote_events += 1,
                MarketEventKind::Bbo { .. } => state.replay_diagnostics.bbo_events += 1,
                MarketEventKind::Trade { .. } => state.replay_diagnostics.trade_events += 1,
                MarketEventKind::Funding { rate } => {
                    state.replay_diagnostics.funding_events += 1;
                    ExecutionModel::apply_event_funding(&mut state, &venue_snapshot, *rate);
                }
                MarketEventKind::Depth { .. } => state.replay_diagnostics.depth_events += 1,
                MarketEventKind::Candle { candle } => {
                    state.replay_diagnostics.candle_events += 1;
                    candle_events.push(candle.clone());
                }
            }

            if let Some(intent) = pending_order.take() {
                pending_order = execution_model.process_event_pending_order(
                    &mut state,
                    intent,
                    &venue_snapshot,
                    event,
                    index,
                    &request.backtest_config,
                    &request.execution_config,
                );
            }
            ExecutionModel::maybe_liquidate_snapshot(
                &mut state,
                &venue_snapshot,
                &request.execution_config,
            );
            if let Some(mark_price) = venue_snapshot.mark_price() {
                Self::record_equity_point(
                    &mut state,
                    event.timestamp,
                    mark_price,
                    &mut equity_curve,
                    &mut drawdown_curve,
                    &mut peak_equity,
                );
            }

            while signal_cursor < signals.len()
                && signals[signal_cursor].timestamp <= event.timestamp
            {
                Self::schedule_signal(
                    &mut state,
                    &mut pending_order,
                    signals[signal_cursor].clone(),
                    index,
                    request,
                    event.timestamp,
                    events.len(),
                    true,
                );
                signal_cursor += 1;
            }

            if let (Some(strategy), MarketEventKind::Candle { .. }) =
                (rule_strategy.as_mut(), &event.kind)
            {
                let position_state = state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat);
                if let Some(signal) = strategy.on_candle(
                    candle_events.len().saturating_sub(1),
                    &candle_events,
                    position_state,
                ) {
                    if !matches!(signal.action, SignalAction::Hold) {
                        Self::schedule_signal(
                            &mut state,
                            &mut pending_order,
                            signal,
                            index,
                            request,
                            event.timestamp,
                            events.len(),
                            true,
                        );
                    }
                }
            }
        }

        if let Some(intent) = pending_order.take() {
            let completed_at = events.last().map(|event| event.timestamp).unwrap();
            ExecutionModel::cancel_order(
                &mut state,
                &intent,
                completed_at,
                "replay_ended_before_fill",
            );
        }
        execution_model.force_flatten_snapshot(
            &mut state,
            &venue_snapshot,
            events.len().saturating_sub(1),
            &request.backtest_config,
            &request.execution_config,
        );
        if state.position.is_none() {
            if let Some(last_point) = equity_curve.last_mut() {
                last_point.cash = state.cash;
                last_point.unrealized_pnl = 0.0;
                last_point.equity = state.cash;
                last_point.position_state = PositionState::Flat;
            }
            if let Some(last_drawdown) = drawdown_curve.last_mut() {
                last_drawdown.equity = state.cash;
                last_drawdown.peak_equity = last_drawdown.peak_equity.max(state.cash);
                last_drawdown.drawdown = last_drawdown.peak_equity - state.cash;
                last_drawdown.drawdown_pct = if last_drawdown.peak_equity > 0.0 {
                    last_drawdown.drawdown / last_drawdown.peak_equity
                } else {
                    0.0
                };
            }
        }

        Ok(ReplayArtifacts {
            state,
            equity_curve,
            drawdown_curve,
            assumptions: Self::assumption_log(request, "events"),
        })
    }

    fn prepare_events(events: &[MarketEvent]) -> Result<(Vec<MarketEvent>, usize), BacktestError> {
        let mut indexed_events = events
            .iter()
            .cloned()
            .enumerate()
            .collect::<Vec<(usize, MarketEvent)>>();
        indexed_events.sort_by(|(left_idx, left), (right_idx, right)| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.sequence.cmp(&right.sequence))
                .then_with(|| left_idx.cmp(right_idx))
        });
        let reordered_events = indexed_events
            .iter()
            .enumerate()
            .filter(|(new_index, (original_index, _))| new_index != original_index)
            .count();
        let prepared = indexed_events
            .into_iter()
            .map(|(_, event)| event)
            .collect::<Vec<_>>();
        for window in prepared.windows(2) {
            let left = &window[0];
            let right = &window[1];
            match left.timestamp.cmp(&right.timestamp) {
                Ordering::Greater => {
                    return Err(BacktestError::InvalidData(
                        "event stream failed deterministic ordering".to_string(),
                    ))
                }
                Ordering::Equal if left.sequence >= right.sequence => {
                    return Err(BacktestError::InvalidData(
                        "events with matching timestamps must have strictly increasing sequences"
                            .to_string(),
                    ))
                }
                _ => {}
            }
        }
        for event in &prepared {
            if !event.validate() {
                return Err(BacktestError::InvalidData(format!(
                    "invalid event at {} sequence {}",
                    event.timestamp, event.sequence
                )));
            }
        }
        Ok((prepared, reordered_events))
    }

    fn validate_candles(candles: &[Candle]) -> Result<(), BacktestError> {
        if candles.len() < 3 {
            return Err(BacktestError::InvalidData(
                "at least 3 candles are required to run a backtest".to_string(),
            ));
        }
        let mut last_timestamp: Option<DateTime<Utc>> = None;
        for candle in candles {
            if !candle.validate() {
                return Err(BacktestError::InvalidData(format!(
                    "invalid candle at {}",
                    candle.timestamp
                )));
            }
            if let Some(previous) = last_timestamp {
                if candle.timestamp <= previous {
                    return Err(BacktestError::InvalidData(
                        "candles must be strictly increasing in time".to_string(),
                    ));
                }
            }
            last_timestamp = Some(candle.timestamp);
        }
        Ok(())
    }

    fn record_equity_point(
        state: &mut BacktestState,
        timestamp: DateTime<Utc>,
        mark_price: f64,
        equity_curve: &mut Vec<EquityPoint>,
        drawdown_curve: &mut Vec<DrawdownPoint>,
        peak_equity: &mut f64,
    ) {
        let position_state = state
            .position
            .as_ref()
            .map(|position| position.state())
            .unwrap_or(PositionState::Flat);
        if position_state != PositionState::Flat {
            state.exposed_bars += 1;
        }
        let unrealized = state
            .position
            .as_ref()
            .map(|position| position.unrealized_pnl(mark_price))
            .unwrap_or(0.0);
        let equity = state.cash + unrealized;
        *peak_equity = (*peak_equity).max(equity);
        let drawdown = *peak_equity - equity;
        let drawdown_pct = if *peak_equity > 0.0 {
            drawdown / *peak_equity
        } else {
            0.0
        };
        equity_curve.push(EquityPoint {
            timestamp,
            equity,
            cash: state.cash,
            unrealized_pnl: unrealized,
            position_state,
        });
        drawdown_curve.push(DrawdownPoint {
            timestamp,
            equity,
            peak_equity: *peak_equity,
            drawdown,
            drawdown_pct,
        });
    }

    fn schedule_signal(
        state: &mut BacktestState,
        pending_order: &mut Option<OrderIntent>,
        signal: Signal,
        current_index: usize,
        request: &BacktestRequest,
        current_timestamp: DateTime<Utc>,
        dataset_len: usize,
        event_mode: bool,
    ) {
        if matches!(signal.action, SignalAction::Hold) {
            return;
        }
        if let Some(existing) = pending_order.take() {
            ExecutionModel::cancel_order(
                state,
                &existing,
                current_timestamp,
                "superseded_by_new_signal",
            );
        }
        let latency = if event_mode {
            request.execution_config.latency_events
        } else {
            request.execution_config.latency_bars
        };
        let execute_at_index = current_index.saturating_add(latency + 1);
        let timeout = if event_mode {
            request.execution_config.order_timeout_events
        } else {
            request.execution_config.order_timeout_bars
        };
        let expires_at_index = timeout.map(|timeout| execute_at_index.saturating_add(timeout));
        let requested_kind = match request.execution_config.order_type {
            OrderTypeAssumption::Market => OrderKind::Market,
            OrderTypeAssumption::Limit if signal.limit_price.is_some() => OrderKind::Limit,
            OrderTypeAssumption::Limit => OrderKind::Market,
        };
        let intent = ExecutionModel::build_order_intent(
            state.allocate_order_id(),
            signal.action,
            current_index,
            execute_at_index.min(dataset_len.saturating_sub(1)),
            expires_at_index,
            signal
                .leverage_override
                .unwrap_or(request.backtest_config.default_leverage),
            signal.limit_price,
            signal.note.unwrap_or_else(|| "strategy_signal".to_string()),
            signal.timestamp,
            requested_kind,
        );
        ExecutionModel::record_order_submission(state, &intent);
        *pending_order = Some(intent);
    }

    fn build_manifest(
        request: &BacktestRequest,
        request_signature: &str,
        assumptions: &BTreeMap<String, String>,
    ) -> RunManifest {
        let (range_start, range_end) = request.market_data.time_bounds().unzip();
        let input_summary = BTreeMap::from([
            ("symbol".to_string(), request.context.symbol.clone()),
            ("timeframe".to_string(), request.context.timeframe.clone()),
            (
                "strategy_kind".to_string(),
                strategy_kind_label(&request.strategy_input).to_string(),
            ),
            (
                "data_mode".to_string(),
                request.market_data.mode_label().to_string(),
            ),
        ]);
        RunManifest {
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            artifact_schema_version: ARTIFACT_SCHEMA_VERSION.to_string(),
            request_signature: request_signature.to_string(),
            data_mode: request.market_data.mode_label().to_string(),
            dataset_length: request.market_data.len(),
            range_start,
            range_end,
            input_summary,
            assumption_summary: assumptions.clone(),
        }
    }

    fn assumption_log(request: &BacktestRequest, replay_mode: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "validation_philosophy".to_string(),
                "survive_reality_not_just_profit".to_string(),
            ),
            ("replay_mode".to_string(), replay_mode.to_string()),
            (
                "order_type".to_string(),
                format!("{:?}", request.execution_config.order_type),
            ),
            (
                "market_price_reference".to_string(),
                format!("{:?}", request.execution_config.market_price_reference),
            ),
            (
                "limit_fill_assumption".to_string(),
                format!("{:?}", request.execution_config.limit_fill_assumption),
            ),
            (
                "latency_bars".to_string(),
                request.execution_config.latency_bars.to_string(),
            ),
            (
                "latency_events".to_string(),
                request.execution_config.latency_events.to_string(),
            ),
        ])
    }
}

fn sorted_signal_stream(strategy_input: &StrategyInput) -> Vec<Signal> {
    let StrategyInput::SignalStream(signals) = strategy_input else {
        return Vec::new();
    };
    let mut indexed = signals
        .iter()
        .cloned()
        .enumerate()
        .collect::<Vec<(usize, Signal)>>();
    indexed.sort_by(|(left_idx, left), (right_idx, right)| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left_idx.cmp(right_idx))
    });
    indexed.into_iter().map(|(_, signal)| signal).collect()
}

fn strategy_kind_label(strategy_input: &StrategyInput) -> &'static str {
    match strategy_input {
        StrategyInput::Definition(_) => "definition",
        StrategyInput::SignalStream(_) => "signal_stream",
    }
}
#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::BacktestEngine;
    use crate::domain::config::{
        BacktestConfig, BacktestRequest, ExecutionConfig, LimitFillAssumption, LiquidationConfig,
        MarketPriceReference, OrderTypeAssumption, PositionSizing, RunContext,
    };
    use crate::domain::types::{
        Candle, MarketDataSet, MarketEvent, MarketEventKind, Signal, SignalAction, StrategyInput,
    };

    fn event_request(events: Vec<MarketEvent>) -> BacktestRequest {
        BacktestRequest {
            context: RunContext {
                symbol: "BTC-PERP".to_string(),
                venue: Some("event_fixture".to_string()),
                timeframe: "tick".to_string(),
                run_label: Some("event".to_string()),
            },
            market_data: MarketDataSet::Events(events),
            strategy_input: StrategyInput::SignalStream(vec![Signal {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                action: SignalAction::GoLong,
                leverage_override: Some(1.0),
                limit_price: None,
                note: Some("event_signal".to_string()),
            }]),
            backtest_config: BacktestConfig {
                starting_cash: 1_000.0,
                default_leverage: 1.0,
                max_leverage: 2.0,
                position_sizing: PositionSizing::FixedNotional { notional: 100.0 },
                allow_long: true,
                allow_short: true,
            },
            execution_config: ExecutionConfig {
                taker_fee_bps: 5.0,
                maker_fee_bps: 2.0,
                spread_bps: 2.0,
                slippage_bps: 1.0,
                latency_bars: 0,
                latency_events: 0,
                order_timeout_bars: None,
                order_timeout_events: Some(2),
                order_type: OrderTypeAssumption::Market,
                market_price_reference: MarketPriceReference::OpposingBest,
                limit_fill_assumption: LimitFillAssumption::Touch,
                use_candle_spread: false,
                partial_fill_ratio: None,
                liquidation: LiquidationConfig::default(),
            },
        }
    }

    #[test]
    fn event_replay_orders_by_timestamp_then_sequence() {
        let base = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let events = vec![
            MarketEvent {
                timestamp: base + Duration::seconds(1),
                sequence: 3,
                kind: MarketEventKind::Trade {
                    price: 101.0,
                    quantity: 1.0,
                    aggressor: None,
                },
            },
            MarketEvent {
                timestamp: base,
                sequence: 2,
                kind: MarketEventKind::Quote {
                    bid: 99.9,
                    ask: 100.1,
                    bid_size: None,
                    ask_size: None,
                },
            },
            MarketEvent {
                timestamp: base,
                sequence: 1,
                kind: MarketEventKind::Quote {
                    bid: 99.8,
                    ask: 100.0,
                    bid_size: None,
                    ask_size: None,
                },
            },
        ];
        let report = BacktestEngine::run(event_request(events), 1).unwrap();
        assert_eq!(report.artifacts.replay_diagnostics.reordered_events, 2);
        assert_eq!(report.artifacts.replay_diagnostics.last_sequence, Some(3));
    }

    #[test]
    fn candle_replay_still_runs() {
        let candles = (0..4)
            .map(|index| Candle {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                    + Duration::hours(index as i64),
                open: 100.0 + index as f64,
                high: 101.0 + index as f64,
                low: 99.0 + index as f64,
                close: 100.5 + index as f64,
                volume: 10.0,
                funding_rate: 0.0,
                spread_bps: Some(3.0),
            })
            .collect();
        let request = BacktestRequest {
            context: RunContext {
                symbol: "BTC-PERP".to_string(),
                venue: Some("candle_fixture".to_string()),
                timeframe: "1h".to_string(),
                run_label: Some("candle".to_string()),
            },
            market_data: MarketDataSet::Candles(candles),
            strategy_input: StrategyInput::SignalStream(vec![Signal {
                timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                action: SignalAction::GoLong,
                leverage_override: Some(1.0),
                limit_price: None,
                note: Some("candle_signal".to_string()),
            }]),
            backtest_config: BacktestConfig {
                starting_cash: 1_000.0,
                default_leverage: 1.0,
                max_leverage: 2.0,
                position_sizing: PositionSizing::FixedNotional { notional: 100.0 },
                allow_long: true,
                allow_short: true,
            },
            execution_config: event_request(Vec::new()).execution_config,
        };
        let report = BacktestEngine::run(request, 1).unwrap();
        assert_eq!(report.artifacts.replay_diagnostics.mode, "candles");
        assert!(!report.artifacts.order_audit_log.is_empty());
    }
}
