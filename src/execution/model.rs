use chrono::{DateTime, Utc};

use crate::domain::config::{
    BacktestConfig, ExecutionConfig, LimitFillAssumption, MarketPriceReference, PositionSizing,
};
use crate::domain::types::{
    Candle, ExecutionLeg, ExecutionLegRole, Fill, MarketEvent, MarketEventKind, OrderAuditEntry,
    OrderIntent, OrderKind, OrderStatus, Position, PositionState, Side, SignalAction, Trade,
};
use crate::engine::state::BacktestState;

#[derive(Debug, Clone)]
pub struct ExecutionOutcome {
    pub fills: Vec<Fill>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionVenueSnapshot {
    pub timestamp: Option<DateTime<Utc>>,
    pub sequence: Option<u64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub last_trade_price: Option<f64>,
    pub candle_open: Option<f64>,
    pub candle_high: Option<f64>,
    pub candle_low: Option<f64>,
    pub candle_close: Option<f64>,
}

impl ExecutionVenueSnapshot {
    pub fn from_candle(candle: &Candle) -> Self {
        Self {
            timestamp: Some(candle.timestamp),
            sequence: None,
            best_bid: None,
            best_ask: None,
            last_trade_price: Some(candle.close),
            candle_open: Some(candle.open),
            candle_high: Some(candle.high),
            candle_low: Some(candle.low),
            candle_close: Some(candle.close),
        }
    }

    pub fn apply_event(&mut self, event: &MarketEvent) {
        self.timestamp = Some(event.timestamp);
        self.sequence = Some(event.sequence);
        match &event.kind {
            MarketEventKind::Quote { bid, ask, .. } | MarketEventKind::Bbo { bid, ask, .. } => {
                self.best_bid = Some(*bid);
                self.best_ask = Some(*ask);
            }
            MarketEventKind::Trade { price, .. } => {
                self.last_trade_price = Some(*price);
            }
            MarketEventKind::Funding { .. } => {}
            MarketEventKind::Depth {
                side,
                price,
                quantity,
                ..
            } => {
                if *quantity == 0.0 {
                    match side {
                        Side::Long => {
                            if self.best_bid == Some(*price) {
                                self.best_bid = None;
                            }
                        }
                        Side::Short => {
                            if self.best_ask == Some(*price) {
                                self.best_ask = None;
                            }
                        }
                    }
                } else {
                    match side {
                        Side::Long => {
                            self.best_bid = Some(
                                self.best_bid
                                    .map(|current| current.max(*price))
                                    .unwrap_or(*price),
                            );
                        }
                        Side::Short => {
                            self.best_ask = Some(
                                self.best_ask
                                    .map(|current| current.min(*price))
                                    .unwrap_or(*price),
                            );
                        }
                    }
                }
            }
            MarketEventKind::Candle { candle } => {
                self.candle_open = Some(candle.open);
                self.candle_high = Some(candle.high);
                self.candle_low = Some(candle.low);
                self.candle_close = Some(candle.close);
                self.last_trade_price = Some(candle.close);
            }
        }
    }

    pub fn mid_price(&self) -> Option<f64> {
        Some((self.best_bid? + self.best_ask?) / 2.0)
    }

    pub fn mark_price(&self) -> Option<f64> {
        self.mid_price()
            .or(self.last_trade_price)
            .or(self.candle_close)
            .or(self.candle_open)
            .or(self.best_bid)
            .or(self.best_ask)
    }

    pub fn liquidation_mark_price(&self, side: Side) -> Option<f64> {
        match side {
            Side::Long => self
                .best_bid
                .or(self.last_trade_price)
                .or(self.candle_low)
                .or(self.mark_price()),
            Side::Short => self
                .best_ask
                .or(self.last_trade_price)
                .or(self.candle_high)
                .or(self.mark_price()),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FillPricing {
    reference_price: f64,
    realized_execution_price: f64,
    fee_rate_bps: f64,
    spread_bps: f64,
    slippage_bps: f64,
    maker: bool,
    kind: OrderKind,
}

#[derive(Debug, Default, Clone)]
pub struct ExecutionModel;

impl ExecutionModel {
    pub fn build_order_intent(
        order_id: u64,
        action: SignalAction,
        generated_index: usize,
        execute_at_index: usize,
        expires_at_index: Option<usize>,
        leverage: f64,
        limit_price: Option<f64>,
        note: String,
        timestamp: DateTime<Utc>,
        requested_kind: OrderKind,
    ) -> OrderIntent {
        OrderIntent {
            order_id,
            generated_at: timestamp,
            generated_index,
            execute_at_index,
            expires_at_index,
            action,
            requested_kind,
            leverage,
            limit_price,
            note,
        }
    }

    pub fn record_order_submission(state: &mut BacktestState, intent: &OrderIntent) {
        state.execution_diagnostics.submitted_orders += 1;
        state.order_audit_log.push(OrderAuditEntry {
            order_id: intent.order_id,
            generated_at: intent.generated_at,
            completed_at: None,
            trigger_sequence: None,
            action: intent.action.clone(),
            requested_kind: intent.requested_kind,
            realized_kind: None,
            status: OrderStatus::Submitted,
            limit_price: intent.limit_price,
            fill_price: None,
            quantity: None,
            maker: None,
            fill_delay_steps: None,
            fill_delay_seconds: None,
            execution_leg_count: None,
            position_before: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            position_after: None,
            reason_code: "submitted".to_string(),
            reason: intent.note.clone(),
        });
    }

    pub fn cancel_order(
        state: &mut BacktestState,
        intent: &OrderIntent,
        completed_at: DateTime<Utc>,
        reason: &str,
    ) {
        state.execution_diagnostics.cancelled_orders += 1;
        state.record_reason(reason);
        state.order_audit_log.push(OrderAuditEntry {
            order_id: intent.order_id,
            generated_at: intent.generated_at,
            completed_at: Some(completed_at),
            trigger_sequence: None,
            action: intent.action.clone(),
            requested_kind: intent.requested_kind,
            realized_kind: None,
            status: OrderStatus::Cancelled,
            limit_price: intent.limit_price,
            fill_price: None,
            quantity: None,
            maker: None,
            fill_delay_steps: None,
            fill_delay_seconds: None,
            execution_leg_count: None,
            position_before: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            position_after: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            reason_code: reason.to_string(),
            reason: reason.to_string(),
        });
    }

    pub fn expire_order(
        state: &mut BacktestState,
        intent: &OrderIntent,
        completed_at: DateTime<Utc>,
        reason: &str,
    ) {
        state.execution_diagnostics.expired_orders += 1;
        state.record_reason(reason);
        state.order_audit_log.push(OrderAuditEntry {
            order_id: intent.order_id,
            generated_at: intent.generated_at,
            completed_at: Some(completed_at),
            trigger_sequence: None,
            action: intent.action.clone(),
            requested_kind: intent.requested_kind,
            realized_kind: None,
            status: OrderStatus::Expired,
            limit_price: intent.limit_price,
            fill_price: None,
            quantity: None,
            maker: None,
            fill_delay_steps: None,
            fill_delay_seconds: None,
            execution_leg_count: None,
            position_before: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            position_after: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            reason_code: reason.to_string(),
            reason: reason.to_string(),
        });
    }

    pub fn reject_order(
        state: &mut BacktestState,
        intent: &OrderIntent,
        completed_at: DateTime<Utc>,
        reason: &str,
    ) {
        state.execution_diagnostics.rejected_orders += 1;
        state.record_reason(reason);
        state.order_audit_log.push(OrderAuditEntry {
            order_id: intent.order_id,
            generated_at: intent.generated_at,
            completed_at: Some(completed_at),
            trigger_sequence: None,
            action: intent.action.clone(),
            requested_kind: intent.requested_kind,
            realized_kind: None,
            status: OrderStatus::Rejected,
            limit_price: intent.limit_price,
            fill_price: None,
            quantity: None,
            maker: None,
            fill_delay_steps: None,
            fill_delay_seconds: None,
            execution_leg_count: None,
            position_before: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            position_after: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            reason_code: reason.to_string(),
            reason: reason.to_string(),
        });
    }

    pub fn apply_candle_funding(state: &mut BacktestState, candle: &Candle) {
        Self::apply_funding_rate(state, candle.close, candle.funding_rate);
    }

    pub fn apply_event_funding(
        state: &mut BacktestState,
        snapshot: &ExecutionVenueSnapshot,
        rate: f64,
    ) {
        if let Some(mark_price) = snapshot.mark_price() {
            Self::apply_funding_rate(state, mark_price, rate);
        }
    }

    pub fn maybe_liquidate_candle(
        state: &mut BacktestState,
        candle: &Candle,
        execution_config: &ExecutionConfig,
    ) {
        let Some(position) = state.position.clone() else {
            return;
        };
        let mark_price = if execution_config.liquidation.use_adverse_price_path {
            match position.side {
                Side::Long => candle.low,
                Side::Short => candle.high,
            }
        } else {
            candle.close
        };
        Self::maybe_liquidate_mark(
            state,
            candle.timestamp,
            None,
            mark_price,
            execution_config,
            "maintenance_margin_breach",
        );
    }

    pub fn maybe_liquidate_snapshot(
        state: &mut BacktestState,
        snapshot: &ExecutionVenueSnapshot,
        execution_config: &ExecutionConfig,
    ) {
        let Some(position) = state.position.clone() else {
            return;
        };
        let Some(timestamp) = snapshot.timestamp else {
            return;
        };
        let Some(mark_price) = snapshot.liquidation_mark_price(position.side) else {
            return;
        };
        Self::maybe_liquidate_mark(
            state,
            timestamp,
            snapshot.sequence,
            mark_price,
            execution_config,
            "event_margin_breach",
        );
    }

    pub fn process_candle_pending_order(
        &self,
        state: &mut BacktestState,
        intent: OrderIntent,
        candle: &Candle,
        index: usize,
        backtest_config: &BacktestConfig,
        execution_config: &ExecutionConfig,
    ) -> Option<OrderIntent> {
        if index < intent.execute_at_index {
            return Some(intent);
        }
        if intent
            .expires_at_index
            .map(|expires_at| index > expires_at)
            .unwrap_or(false)
        {
            Self::expire_order(state, &intent, candle.timestamp, "limit_order_timeout");
            return None;
        }
        let snapshot = ExecutionVenueSnapshot::from_candle(candle);
        let order_side = self
            .order_execution_side(&intent, state.position.as_ref())
            .or_else(|| intent.action.target_side());
        let pricing = match intent.requested_kind {
            OrderKind::Market => order_side
                .and_then(|side| self.candle_market_pricing(candle, side, execution_config)),
            OrderKind::Limit => order_side.and_then(|side| {
                self.candle_limit_pricing(candle, &intent, side, execution_config)
            }),
            OrderKind::Liquidation => None,
        };
        if let Some(pricing) = pricing {
            self.execute_rebalance_with_pricing(
                state,
                &intent,
                index,
                candle.timestamp,
                backtest_config,
                pricing,
                snapshot.sequence,
                execution_config.partial_fill_ratio,
            );
            return None;
        }
        if intent
            .expires_at_index
            .map(|expires_at| index >= expires_at)
            .unwrap_or(false)
        {
            Self::expire_order(state, &intent, candle.timestamp, "limit_order_timeout");
            return None;
        }
        Some(intent)
    }

    pub fn process_event_pending_order(
        &self,
        state: &mut BacktestState,
        intent: OrderIntent,
        snapshot: &ExecutionVenueSnapshot,
        event: &MarketEvent,
        index: usize,
        backtest_config: &BacktestConfig,
        execution_config: &ExecutionConfig,
    ) -> Option<OrderIntent> {
        if index < intent.execute_at_index {
            return Some(intent);
        }
        if intent
            .expires_at_index
            .map(|expires_at| index > expires_at)
            .unwrap_or(false)
        {
            Self::expire_order(state, &intent, event.timestamp, "event_order_timeout");
            return None;
        }
        let order_side = self
            .order_execution_side(&intent, state.position.as_ref())
            .or_else(|| intent.action.target_side());
        let pricing = match intent.requested_kind {
            OrderKind::Market => order_side.and_then(|side| {
                self.event_market_pricing(snapshot, side, execution_config, event.sequence)
            }),
            OrderKind::Limit => order_side.and_then(|side| {
                self.event_limit_pricing(snapshot, &intent, side, execution_config, index, event)
            }),
            OrderKind::Liquidation => None,
        };
        if let Some(pricing) = pricing {
            self.execute_rebalance_with_pricing(
                state,
                &intent,
                index,
                event.timestamp,
                backtest_config,
                pricing,
                Some(event.sequence),
                execution_config.partial_fill_ratio,
            );
            return None;
        }
        if intent
            .expires_at_index
            .map(|expires_at| index >= expires_at)
            .unwrap_or(false)
        {
            Self::expire_order(state, &intent, event.timestamp, "event_order_timeout");
            return None;
        }
        Some(intent)
    }

    pub fn force_flatten_candle(
        &self,
        state: &mut BacktestState,
        candle: &Candle,
        index: usize,
        backtest_config: &BacktestConfig,
        execution_config: &ExecutionConfig,
    ) {
        if state.position.is_none() {
            return;
        }
        let intent = Self::build_order_intent(
            state.allocate_order_id(),
            SignalAction::ExitToFlat,
            index,
            index,
            None,
            1.0,
            None,
            "end_of_backtest".to_string(),
            candle.timestamp,
            OrderKind::Market,
        );
        Self::record_order_submission(state, &intent);
        let order_side = self.order_execution_side(&intent, state.position.as_ref());
        if let Some(pricing) =
            order_side.and_then(|side| self.candle_market_pricing(candle, side, execution_config))
        {
            self.execute_rebalance_with_pricing(
                state,
                &intent,
                index,
                candle.timestamp,
                backtest_config,
                pricing,
                None,
                execution_config.partial_fill_ratio,
            );
        }
    }

    pub fn force_flatten_snapshot(
        &self,
        state: &mut BacktestState,
        snapshot: &ExecutionVenueSnapshot,
        index: usize,
        backtest_config: &BacktestConfig,
        execution_config: &ExecutionConfig,
    ) {
        if state.position.is_none() {
            return;
        }
        let Some(timestamp) = snapshot.timestamp else {
            return;
        };
        let intent = Self::build_order_intent(
            state.allocate_order_id(),
            SignalAction::ExitToFlat,
            index,
            index,
            None,
            1.0,
            None,
            "end_of_backtest".to_string(),
            timestamp,
            OrderKind::Market,
        );
        Self::record_order_submission(state, &intent);
        let order_side = self.order_execution_side(&intent, state.position.as_ref());
        if let Some(pricing) = order_side.and_then(|side| {
            self.event_market_pricing(
                snapshot,
                side,
                execution_config,
                snapshot.sequence.unwrap_or(0),
            )
        }) {
            self.execute_rebalance_with_pricing(
                state,
                &intent,
                index,
                timestamp,
                backtest_config,
                pricing,
                snapshot.sequence,
                execution_config.partial_fill_ratio,
            );
        } else {
            Self::reject_order(
                state,
                &intent,
                timestamp,
                "no_market_price_for_force_flatten",
            );
        }
    }

    fn apply_funding_rate(state: &mut BacktestState, mark_price: f64, rate: f64) {
        if rate == 0.0 {
            return;
        }
        if let Some(position) = state.position.as_mut() {
            let signed_notional = position.signed_quantity() * mark_price;
            let funding_cash = -signed_notional * rate;
            state.cash += funding_cash;
            state.funding_impact += -funding_cash;
            state.execution_diagnostics.funding_events_applied += 1;
            position.accumulated_funding += -funding_cash;
        }
    }

    fn maybe_liquidate_mark(
        state: &mut BacktestState,
        timestamp: DateTime<Utc>,
        sequence: Option<u64>,
        mark_price: f64,
        execution_config: &ExecutionConfig,
        note: &str,
    ) {
        let Some(position) = state.position.clone() else {
            return;
        };
        let unrealized = position.unrealized_pnl(mark_price);
        let equity = state.cash + unrealized;
        let maintenance_margin =
            position.notional(mark_price) * execution_config.liquidation.maintenance_margin_ratio;
        if equity > maintenance_margin {
            return;
        }

        let liquidation_fee_rate = execution_config.liquidation.liquidation_fee_bps / 10_000.0;
        let fee = position.notional(mark_price) * liquidation_fee_rate;
        let gross_pnl = position.unrealized_pnl(mark_price);
        let net_pnl = gross_pnl - fee - position.accumulated_funding - position.accumulated_fees;

        state.cash += gross_pnl - fee;
        state.fee_impact += fee;
        state.execution_diagnostics.total_liquidation_fees += fee;
        state.execution_diagnostics.liquidation_count += 1;
        state.execution_diagnostics.liquidation_legs += 1;
        state.record_reason(note);
        let fill_delay_steps = 0;
        let fill = Fill {
            order_id: 0,
            submitted_at: position.opened_at,
            timestamp,
            trigger_sequence: sequence,
            kind: OrderKind::Liquidation,
            side: position.side.opposite(),
            quantity: position.quantity,
            reference_price: mark_price,
            realized_execution_price: mark_price,
            fee_paid: fee,
            spread_cost: 0.0,
            slippage_cost: 0.0,
            liquidation: true,
            maker: false,
            fill_delay_steps,
            fill_delay_seconds: 0,
            note: note.to_string(),
        };
        state.fills.push(fill);
        state.execution_legs.push(ExecutionLeg {
            order_id: 0,
            leg_index: 1,
            role: ExecutionLegRole::Liquidation,
            generated_at: position.opened_at,
            executed_at: timestamp,
            trigger_sequence: sequence,
            requested_kind: OrderKind::Liquidation,
            realized_kind: OrderKind::Liquidation,
            side: position.side.opposite(),
            quantity: position.quantity,
            reference_price: mark_price,
            realized_execution_price: mark_price,
            maker: false,
            fee_paid: fee,
            spread_cost: 0.0,
            slippage_cost: 0.0,
            funding_cost: position.accumulated_funding,
            gross_pnl: Some(gross_pnl),
            net_pnl: Some(net_pnl),
            fill_delay_steps: 0,
            fill_delay_seconds: 0,
            position_before: position.state(),
            position_after: PositionState::Flat,
            liquidation: true,
            reason_code: note.to_string(),
            reason: "forced liquidation".to_string(),
        });
        state.trades.push(Trade {
            entry_time: position.opened_at,
            exit_time: timestamp,
            side: position.side,
            quantity: position.quantity,
            leverage: position.leverage,
            entry_price: position.entry_price,
            exit_price: mark_price,
            gross_pnl,
            net_pnl,
            fees_paid: position.accumulated_fees + fee,
            funding_paid: position.accumulated_funding,
            slippage_paid: position.accumulated_slippage,
            spread_paid: position.accumulated_spread,
            duration_seconds: timestamp
                .signed_duration_since(position.opened_at)
                .num_seconds(),
            liquidated: true,
            entry_note: None,
            exit_note: Some("forced_liquidation".to_string()),
        });
        state.order_audit_log.push(OrderAuditEntry {
            order_id: 0,
            generated_at: position.opened_at,
            completed_at: Some(timestamp),
            trigger_sequence: sequence,
            action: SignalAction::ExitToFlat,
            requested_kind: OrderKind::Liquidation,
            realized_kind: Some(OrderKind::Liquidation),
            status: OrderStatus::Filled,
            limit_price: None,
            fill_price: Some(mark_price),
            quantity: Some(position.quantity),
            maker: Some(false),
            fill_delay_steps: Some(0),
            fill_delay_seconds: Some(0),
            execution_leg_count: Some(1),
            position_before: Some(position.state()),
            position_after: Some(PositionState::Flat),
            reason_code: note.to_string(),
            reason: "liquidated".to_string(),
        });
        state.position = None;
    }

    fn execute_rebalance_with_pricing(
        &self,
        state: &mut BacktestState,
        intent: &OrderIntent,
        index: usize,
        timestamp: DateTime<Utc>,
        backtest_config: &BacktestConfig,
        pricing: FillPricing,
        trigger_sequence: Option<u64>,
        partial_fill_ratio: Option<f64>,
    ) {
        let target = self.target_from_action(
            intent.action.clone(),
            pricing.reference_price,
            state.cash,
            intent.leverage,
            backtest_config,
        );
        let position_before = state
            .position
            .as_ref()
            .map(|position| position.state())
            .unwrap_or(PositionState::Flat);
        let is_flip = position_before != PositionState::Flat
            && matches!(intent.action, SignalAction::GoLong | SignalAction::GoShort);
        let mut fills = Vec::new();
        if state.position.is_some() {
            if matches!(intent.action, SignalAction::ExitToFlat)
                || state.position.as_ref().map(|p| p.side) != target.as_ref().map(|(s, _, _)| *s)
            {
                if let Some(fill) =
                    self.close_position(state, intent, timestamp, index, pricing, trigger_sequence)
                {
                    fills.push(fill);
                }
            }
        }
        if let Some((side, quantity, leverage)) = target {
            if quantity > 0.0
                && state
                    .position
                    .as_ref()
                    .map(|p| p.side != side || (p.quantity - quantity).abs() > 1e-12)
                    .unwrap_or(true)
            {
                if let Some(fill) = self.open_position(
                    state,
                    intent,
                    side,
                    quantity,
                    leverage,
                    timestamp,
                    index,
                    pricing,
                    trigger_sequence,
                    partial_fill_ratio,
                    is_flip,
                ) {
                    fills.push(fill);
                }
            }
        }

        if fills.is_empty() {
            Self::reject_order(
                state,
                intent,
                timestamp,
                "no_position_change_generated_by_signal",
            );
            return;
        }
        state.execution_diagnostics.filled_orders += 1;
        if fills.len() > 1 {
            state.execution_diagnostics.flip_orders += 1;
            state.record_reason("position_flip");
        } else {
            state.record_reason("filled");
        }
        if pricing.kind == OrderKind::Market {
            state.execution_diagnostics.market_fills += 1;
        } else if pricing.maker {
            state.execution_diagnostics.maker_limit_fills += 1;
        } else {
            state.execution_diagnostics.taker_limit_fills += 1;
        }
        state.record_fill_delay(
            index.saturating_sub(intent.generated_index),
            timestamp
                .signed_duration_since(intent.generated_at)
                .num_seconds(),
        );
        state.order_audit_log.push(OrderAuditEntry {
            order_id: intent.order_id,
            generated_at: intent.generated_at,
            completed_at: Some(timestamp),
            trigger_sequence,
            action: intent.action.clone(),
            requested_kind: intent.requested_kind,
            realized_kind: Some(pricing.kind),
            status: OrderStatus::Filled,
            limit_price: intent.limit_price,
            fill_price: if fills.len() == 1 {
                fills.last().map(|fill| fill.realized_execution_price)
            } else {
                None
            },
            quantity: if fills.len() == 1 {
                Some(fills[0].quantity)
            } else {
                None
            },
            maker: if fills.len() == 1 {
                Some(pricing.maker)
            } else {
                None
            },
            fill_delay_steps: Some(index.saturating_sub(intent.generated_index)),
            fill_delay_seconds: Some(
                timestamp
                    .signed_duration_since(intent.generated_at)
                    .num_seconds(),
            ),
            execution_leg_count: Some(fills.len()),
            position_before: Some(position_before),
            position_after: Some(
                state
                    .position
                    .as_ref()
                    .map(|position| position.state())
                    .unwrap_or(PositionState::Flat),
            ),
            reason_code: if fills.len() > 1 {
                "position_flip".to_string()
            } else {
                "filled".to_string()
            },
            reason: intent.note.clone(),
        });
        state.fills.extend(fills);
    }

    fn target_from_action(
        &self,
        action: SignalAction,
        reference_price: f64,
        cash: f64,
        leverage: f64,
        backtest_config: &BacktestConfig,
    ) -> Option<(Side, f64, f64)> {
        let side = match action {
            SignalAction::GoLong if backtest_config.allow_long => Side::Long,
            SignalAction::GoShort if backtest_config.allow_short => Side::Short,
            SignalAction::ExitToFlat | SignalAction::Hold => return None,
            _ => return None,
        };
        let leverage = leverage.min(backtest_config.max_leverage).max(1.0);
        let max_notional = cash.max(0.0) * backtest_config.max_leverage;
        let raw_notional = match backtest_config.position_sizing {
            PositionSizing::PercentOfEquity { fraction } => cash.max(0.0) * fraction * leverage,
            PositionSizing::FixedNotional { notional } => notional,
            PositionSizing::FixedQuantity { quantity } => quantity * reference_price,
        };
        let notional = raw_notional.min(max_notional);
        let quantity = match backtest_config.position_sizing {
            PositionSizing::FixedQuantity { quantity } => quantity,
            _ => {
                if reference_price <= 0.0 {
                    0.0
                } else {
                    notional / reference_price
                }
            }
        };
        Some((side, quantity, leverage))
    }

    fn close_position(
        &self,
        state: &mut BacktestState,
        intent: &OrderIntent,
        timestamp: DateTime<Utc>,
        index: usize,
        pricing: FillPricing,
        trigger_sequence: Option<u64>,
    ) -> Option<Fill> {
        let position = state.position.take()?;
        let leg_index = next_leg_index(state, intent.order_id);
        let notional = position.quantity * pricing.realized_execution_price;
        let fee = notional * pricing.fee_rate_bps / 10_000.0;
        let spread_cost = notional * pricing.spread_bps / 10_000.0;
        let slippage_cost = notional * pricing.slippage_bps / 10_000.0;
        let gross_pnl = position.quantity
            * (pricing.realized_execution_price - position.entry_price)
            * position.side.sign();
        let net_pnl = gross_pnl - fee - position.accumulated_funding - position.accumulated_fees;
        state.cash += gross_pnl - fee;
        state.fee_impact += fee;
        state.spread_impact += spread_cost;
        state.slippage_impact += slippage_cost;
        state.execution_diagnostics.close_legs += 1;
        let fill = Fill {
            order_id: intent.order_id,
            submitted_at: intent.generated_at,
            timestamp,
            trigger_sequence,
            kind: pricing.kind,
            side: position.side.opposite(),
            quantity: position.quantity,
            reference_price: pricing.reference_price,
            realized_execution_price: pricing.realized_execution_price,
            fee_paid: fee,
            spread_cost,
            slippage_cost,
            liquidation: false,
            maker: pricing.maker,
            fill_delay_steps: index.saturating_sub(intent.generated_index),
            fill_delay_seconds: timestamp
                .signed_duration_since(intent.generated_at)
                .num_seconds(),
            note: format!("close: {}", intent.note),
        };
        state.execution_legs.push(ExecutionLeg {
            order_id: intent.order_id,
            leg_index,
            role: ExecutionLegRole::Close,
            generated_at: intent.generated_at,
            executed_at: timestamp,
            trigger_sequence,
            requested_kind: intent.requested_kind,
            realized_kind: pricing.kind,
            side: position.side.opposite(),
            quantity: position.quantity,
            reference_price: pricing.reference_price,
            realized_execution_price: pricing.realized_execution_price,
            maker: pricing.maker,
            fee_paid: fee,
            spread_cost,
            slippage_cost,
            funding_cost: position.accumulated_funding,
            gross_pnl: Some(gross_pnl),
            net_pnl: Some(net_pnl),
            fill_delay_steps: index.saturating_sub(intent.generated_index),
            fill_delay_seconds: timestamp
                .signed_duration_since(intent.generated_at)
                .num_seconds(),
            position_before: position.state(),
            position_after: PositionState::Flat,
            liquidation: false,
            reason_code: if matches!(intent.action, SignalAction::ExitToFlat) {
                "position_close".to_string()
            } else {
                "position_flip_close".to_string()
            },
            reason: intent.note.clone(),
        });
        state.trades.push(Trade {
            entry_time: position.opened_at,
            exit_time: timestamp,
            side: position.side,
            quantity: position.quantity,
            leverage: position.leverage,
            entry_price: position.entry_price,
            exit_price: pricing.realized_execution_price,
            gross_pnl,
            net_pnl,
            fees_paid: position.accumulated_fees + fee,
            funding_paid: position.accumulated_funding,
            slippage_paid: position.accumulated_slippage + slippage_cost,
            spread_paid: position.accumulated_spread + spread_cost,
            duration_seconds: timestamp
                .signed_duration_since(position.opened_at)
                .num_seconds(),
            liquidated: false,
            entry_note: None,
            exit_note: Some(intent.note.clone()),
        });
        Some(fill)
    }

    fn open_position(
        &self,
        state: &mut BacktestState,
        intent: &OrderIntent,
        side: Side,
        quantity: f64,
        leverage: f64,
        timestamp: DateTime<Utc>,
        index: usize,
        pricing: FillPricing,
        trigger_sequence: Option<u64>,
        partial_fill_ratio: Option<f64>,
        is_flip: bool,
    ) -> Option<Fill> {
        if quantity <= 0.0 {
            return None;
        }
        let leg_index = next_leg_index(state, intent.order_id);
        let adjusted_quantity = quantity * partial_fill_ratio.unwrap_or(1.0);
        if adjusted_quantity <= 0.0 {
            return None;
        }
        let position_before = state
            .position
            .as_ref()
            .map(|position| position.state())
            .unwrap_or(PositionState::Flat);
        let notional = adjusted_quantity * pricing.realized_execution_price;
        let fee = notional * pricing.fee_rate_bps / 10_000.0;
        let spread_cost = notional * pricing.spread_bps / 10_000.0;
        let slippage_cost = notional * pricing.slippage_bps / 10_000.0;
        state.cash -= fee;
        state.fee_impact += fee;
        state.spread_impact += spread_cost;
        state.slippage_impact += slippage_cost;
        state.execution_diagnostics.open_legs += 1;
        state.position = Some(Position {
            side,
            quantity: adjusted_quantity,
            entry_price: pricing.realized_execution_price,
            leverage,
            opened_at: timestamp,
            margin_allocated: notional / leverage.max(1.0),
            accumulated_fees: fee,
            accumulated_funding: 0.0,
            accumulated_slippage: slippage_cost,
            accumulated_spread: spread_cost,
        });
        let fill = Fill {
            order_id: intent.order_id,
            submitted_at: intent.generated_at,
            timestamp,
            trigger_sequence,
            kind: pricing.kind,
            side,
            quantity: adjusted_quantity,
            reference_price: pricing.reference_price,
            realized_execution_price: pricing.realized_execution_price,
            fee_paid: fee,
            spread_cost,
            slippage_cost,
            liquidation: false,
            maker: pricing.maker,
            fill_delay_steps: index.saturating_sub(intent.generated_index),
            fill_delay_seconds: timestamp
                .signed_duration_since(intent.generated_at)
                .num_seconds(),
            note: format!("open: {}", intent.note),
        };
        state.execution_legs.push(ExecutionLeg {
            order_id: intent.order_id,
            leg_index,
            role: ExecutionLegRole::Open,
            generated_at: intent.generated_at,
            executed_at: timestamp,
            trigger_sequence,
            requested_kind: intent.requested_kind,
            realized_kind: pricing.kind,
            side,
            quantity: adjusted_quantity,
            reference_price: pricing.reference_price,
            realized_execution_price: pricing.realized_execution_price,
            maker: pricing.maker,
            fee_paid: fee,
            spread_cost,
            slippage_cost,
            funding_cost: 0.0,
            gross_pnl: None,
            net_pnl: None,
            fill_delay_steps: index.saturating_sub(intent.generated_index),
            fill_delay_seconds: timestamp
                .signed_duration_since(intent.generated_at)
                .num_seconds(),
            position_before,
            position_after: side.into(),
            liquidation: false,
            reason_code: if is_flip {
                "position_flip_open".to_string()
            } else {
                "position_open".to_string()
            },
            reason: intent.note.clone(),
        });
        Some(fill)
    }

    fn order_execution_side(
        &self,
        intent: &OrderIntent,
        position: Option<&Position>,
    ) -> Option<Side> {
        match intent.action {
            SignalAction::GoLong => Some(Side::Long),
            SignalAction::GoShort => Some(Side::Short),
            SignalAction::ExitToFlat => position.map(|position| position.side.opposite()),
            SignalAction::Hold => None,
        }
    }

    fn candle_market_pricing(
        &self,
        candle: &Candle,
        side: Side,
        execution_config: &ExecutionConfig,
    ) -> Option<FillPricing> {
        let spread_bps = if execution_config.use_candle_spread {
            candle.spread_bps.unwrap_or(execution_config.spread_bps) / 2.0
        } else {
            execution_config.spread_bps / 2.0
        };
        let reference_price = candle.open;
        Some(FillPricing {
            reference_price,
            realized_execution_price: adjust_execution_price(
                reference_price,
                side,
                spread_bps + execution_config.slippage_bps,
            ),
            fee_rate_bps: execution_config.taker_fee_bps,
            spread_bps,
            slippage_bps: execution_config.slippage_bps,
            maker: false,
            kind: OrderKind::Market,
        })
    }

    fn candle_limit_pricing(
        &self,
        candle: &Candle,
        intent: &OrderIntent,
        side: Side,
        execution_config: &ExecutionConfig,
    ) -> Option<FillPricing> {
        let limit_price = intent.limit_price?;
        let aggressive = match side {
            Side::Long => limit_price >= candle.open,
            Side::Short => limit_price <= candle.open,
        };
        if aggressive {
            let spread_bps = if execution_config.use_candle_spread {
                candle.spread_bps.unwrap_or(execution_config.spread_bps) / 2.0
            } else {
                execution_config.spread_bps / 2.0
            };
            return Some(FillPricing {
                reference_price: candle.open,
                realized_execution_price: adjust_execution_price(
                    candle.open,
                    side,
                    spread_bps + execution_config.slippage_bps,
                ),
                fee_rate_bps: execution_config.taker_fee_bps,
                spread_bps,
                slippage_bps: execution_config.slippage_bps,
                maker: false,
                kind: OrderKind::Limit,
            });
        }
        let touched = match (side, execution_config.limit_fill_assumption) {
            (Side::Long, LimitFillAssumption::Touch) => candle.low <= limit_price,
            (Side::Long, LimitFillAssumption::Through) => candle.low < limit_price,
            (Side::Short, LimitFillAssumption::Touch) => candle.high >= limit_price,
            (Side::Short, LimitFillAssumption::Through) => candle.high > limit_price,
        };
        if !touched {
            return None;
        }
        Some(FillPricing {
            reference_price: limit_price,
            realized_execution_price: limit_price,
            fee_rate_bps: execution_config.maker_fee_bps,
            spread_bps: 0.0,
            slippage_bps: 0.0,
            maker: true,
            kind: OrderKind::Limit,
        })
    }

    fn event_market_pricing(
        &self,
        snapshot: &ExecutionVenueSnapshot,
        side: Side,
        execution_config: &ExecutionConfig,
        _trigger_sequence: u64,
    ) -> Option<FillPricing> {
        let reference_price =
            market_reference_price(snapshot, side, execution_config.market_price_reference)?;
        let spread_bps =
            implied_spread_bps(snapshot, side, execution_config.market_price_reference);
        Some(FillPricing {
            reference_price,
            realized_execution_price: adjust_execution_price(
                reference_price,
                side,
                spread_bps + execution_config.slippage_bps,
            ),
            fee_rate_bps: execution_config.taker_fee_bps,
            spread_bps,
            slippage_bps: execution_config.slippage_bps,
            maker: false,
            kind: OrderKind::Market,
        })
    }

    fn event_limit_pricing(
        &self,
        snapshot: &ExecutionVenueSnapshot,
        intent: &OrderIntent,
        side: Side,
        execution_config: &ExecutionConfig,
        index: usize,
        event: &MarketEvent,
    ) -> Option<FillPricing> {
        let limit_price = intent.limit_price?;
        let marketable_now = match side {
            Side::Long => snapshot
                .best_ask
                .map(|ask| ask <= limit_price)
                .unwrap_or(false),
            Side::Short => snapshot
                .best_bid
                .map(|bid| bid >= limit_price)
                .unwrap_or(false),
        };
        if marketable_now && index == intent.execute_at_index {
            let reference_price =
                market_reference_price(snapshot, side, MarketPriceReference::OpposingBest)?;
            return Some(FillPricing {
                reference_price,
                realized_execution_price: adjust_execution_price(
                    reference_price,
                    side,
                    execution_config.slippage_bps,
                ),
                fee_rate_bps: execution_config.taker_fee_bps,
                spread_bps: 0.0,
                slippage_bps: execution_config.slippage_bps,
                maker: false,
                kind: OrderKind::Limit,
            });
        }

        let touched = match (&event.kind, side, execution_config.limit_fill_assumption) {
            (MarketEventKind::Trade { price, .. }, Side::Long, LimitFillAssumption::Touch) => {
                *price <= limit_price
            }
            (MarketEventKind::Trade { price, .. }, Side::Long, LimitFillAssumption::Through) => {
                *price < limit_price
            }
            (MarketEventKind::Trade { price, .. }, Side::Short, LimitFillAssumption::Touch) => {
                *price >= limit_price
            }
            (MarketEventKind::Trade { price, .. }, Side::Short, LimitFillAssumption::Through) => {
                *price > limit_price
            }
            (_, Side::Long, LimitFillAssumption::Touch) => snapshot
                .best_ask
                .map(|ask| ask <= limit_price)
                .unwrap_or(false),
            (_, Side::Long, LimitFillAssumption::Through) => snapshot
                .best_ask
                .map(|ask| ask < limit_price)
                .unwrap_or(false),
            (_, Side::Short, LimitFillAssumption::Touch) => snapshot
                .best_bid
                .map(|bid| bid >= limit_price)
                .unwrap_or(false),
            (_, Side::Short, LimitFillAssumption::Through) => snapshot
                .best_bid
                .map(|bid| bid > limit_price)
                .unwrap_or(false),
        };
        if !touched {
            return None;
        }

        let reference_price = match side {
            Side::Long => snapshot
                .best_ask
                .map(|ask| ask.min(limit_price))
                .unwrap_or(limit_price),
            Side::Short => snapshot
                .best_bid
                .map(|bid| bid.max(limit_price))
                .unwrap_or(limit_price),
        };
        Some(FillPricing {
            reference_price,
            realized_execution_price: reference_price,
            fee_rate_bps: execution_config.maker_fee_bps,
            spread_bps: 0.0,
            slippage_bps: 0.0,
            maker: true,
            kind: OrderKind::Limit,
        })
    }
}

fn next_leg_index(state: &BacktestState, order_id: u64) -> usize {
    state
        .execution_legs
        .iter()
        .filter(|leg| leg.order_id == order_id)
        .count()
        + 1
}

fn market_reference_price(
    snapshot: &ExecutionVenueSnapshot,
    side: Side,
    reference: MarketPriceReference,
) -> Option<f64> {
    match reference {
        MarketPriceReference::OpposingBest => match side {
            Side::Long => snapshot.best_ask,
            Side::Short => snapshot.best_bid,
        }
        .or(snapshot.mark_price()),
        MarketPriceReference::Mid => snapshot.mid_price().or(snapshot.mark_price()),
        MarketPriceReference::LastTrade => snapshot.last_trade_price.or(snapshot.mark_price()),
        MarketPriceReference::CandleOpen => snapshot.candle_open.or(snapshot.mark_price()),
    }
}

fn event_half_spread_bps(snapshot: &ExecutionVenueSnapshot) -> Option<f64> {
    let bid = snapshot.best_bid?;
    let ask = snapshot.best_ask?;
    let mid = (bid + ask) / 2.0;
    if mid <= 0.0 {
        return None;
    }
    Some(((ask - bid) / mid) * 10_000.0 / 2.0)
}

fn implied_spread_bps(
    snapshot: &ExecutionVenueSnapshot,
    side: Side,
    reference: MarketPriceReference,
) -> f64 {
    match reference {
        MarketPriceReference::OpposingBest => 0.0,
        MarketPriceReference::Mid => event_half_spread_bps(snapshot).unwrap_or(0.0),
        MarketPriceReference::LastTrade | MarketPriceReference::CandleOpen => {
            if let (Some(reference_price), Some(opposing)) = (
                market_reference_price(snapshot, side, reference),
                market_reference_price(snapshot, side, MarketPriceReference::OpposingBest),
            ) {
                if reference_price > 0.0 {
                    ((opposing - reference_price).abs() / reference_price) * 10_000.0
                } else {
                    0.0
                }
            } else {
                0.0
            }
        }
    }
}

fn adjust_execution_price(reference_price: f64, side: Side, cost_bps: f64) -> f64 {
    let multiplier = 1.0 + (cost_bps / 10_000.0) * side.sign();
    reference_price * multiplier
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{ExecutionModel, ExecutionVenueSnapshot};
    use crate::domain::config::{
        BacktestConfig, ExecutionConfig, LimitFillAssumption, LiquidationConfig,
        MarketPriceReference, OrderTypeAssumption, PositionSizing,
    };
    use crate::domain::types::{
        Candle, MarketEvent, MarketEventKind, OrderKind, Position, Side, SignalAction,
    };
    use crate::engine::state::BacktestState;

    fn candle(price: f64) -> Candle {
        Candle {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            open: price,
            high: price * 1.01,
            low: price * 0.99,
            close: price,
            volume: 10.0,
            funding_rate: 0.0,
            spread_bps: Some(4.0),
        }
    }

    fn backtest_config() -> BacktestConfig {
        BacktestConfig {
            starting_cash: 1_000.0,
            default_leverage: 2.0,
            max_leverage: 3.0,
            position_sizing: PositionSizing::FixedNotional { notional: 500.0 },
            allow_long: true,
            allow_short: true,
        }
    }

    fn execution_config() -> ExecutionConfig {
        ExecutionConfig {
            taker_fee_bps: 10.0,
            maker_fee_bps: 2.0,
            spread_bps: 4.0,
            slippage_bps: 6.0,
            latency_bars: 0,
            latency_events: 0,
            order_timeout_bars: Some(2),
            order_timeout_events: Some(3),
            order_type: OrderTypeAssumption::Market,
            market_price_reference: MarketPriceReference::Mid,
            limit_fill_assumption: LimitFillAssumption::Touch,
            use_candle_spread: true,
            partial_fill_ratio: None,
            liquidation: LiquidationConfig::default(),
        }
    }

    #[test]
    fn applies_market_execution_costs_via_realized_price() {
        let mut state = BacktestState {
            cash: 1_000.0,
            ..BacktestState::default()
        };
        let model = ExecutionModel;
        let candle = candle(100.0);
        let intent = ExecutionModel::build_order_intent(
            1,
            SignalAction::GoLong,
            0,
            0,
            None,
            2.0,
            None,
            "test".to_string(),
            candle.timestamp,
            OrderKind::Market,
        );
        ExecutionModel::record_order_submission(&mut state, &intent);
        let pending = model
            .process_candle_pending_order(
                &mut state,
                intent,
                &candle,
                0,
                &backtest_config(),
                &execution_config(),
            )
            .is_some();
        assert!(!pending);
        let fill = state.fills.first().unwrap();
        assert!(fill.realized_execution_price > fill.reference_price);
        assert!(fill.fee_paid > 0.5);
        assert!((state.cash - 999.4996).abs() < 1e-9);
    }

    #[test]
    fn limit_touch_uses_maker_fee() {
        let mut state = BacktestState {
            cash: 1_000.0,
            ..BacktestState::default()
        };
        let model = ExecutionModel;
        let touch_candle = Candle {
            low: 99.0,
            ..candle(100.0)
        };
        let intent = ExecutionModel::build_order_intent(
            1,
            SignalAction::GoLong,
            0,
            0,
            Some(2),
            2.0,
            Some(99.5),
            "limit".to_string(),
            touch_candle.timestamp,
            OrderKind::Limit,
        );
        ExecutionModel::record_order_submission(&mut state, &intent);
        model.process_candle_pending_order(
            &mut state,
            intent,
            &touch_candle,
            0,
            &backtest_config(),
            &execution_config(),
        );
        let fill = state.fills.first().unwrap();
        assert!(fill.maker);
        assert!((fill.fee_paid - 0.1).abs() < 1e-9);
    }

    #[test]
    fn event_market_fill_observes_quotes_and_delay() {
        let mut state = BacktestState {
            cash: 1_000.0,
            ..BacktestState::default()
        };
        let model = ExecutionModel;
        let mut snapshot = ExecutionVenueSnapshot::default();
        let event = MarketEvent {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 1).unwrap(),
            sequence: 2,
            kind: MarketEventKind::Quote {
                bid: 99.9,
                ask: 100.1,
                bid_size: None,
                ask_size: None,
            },
        };
        snapshot.apply_event(&event);
        let intent = ExecutionModel::build_order_intent(
            1,
            SignalAction::GoLong,
            0,
            1,
            None,
            2.0,
            None,
            "event".to_string(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            OrderKind::Market,
        );
        ExecutionModel::record_order_submission(&mut state, &intent);
        let still_pending = model
            .process_event_pending_order(
                &mut state,
                intent.clone(),
                &snapshot,
                &event,
                0,
                &backtest_config(),
                &execution_config(),
            )
            .is_some();
        assert!(still_pending);
        model.process_event_pending_order(
            &mut state,
            intent,
            &snapshot,
            &event,
            1,
            &backtest_config(),
            &execution_config(),
        );
        assert_eq!(state.fills.len(), 1);
        assert_eq!(state.fills[0].fill_delay_steps, 1);
    }

    #[test]
    fn event_market_with_opposing_best_does_not_report_spread_twice() {
        let mut state = BacktestState {
            cash: 1_000.0,
            ..BacktestState::default()
        };
        let model = ExecutionModel;
        let mut snapshot = ExecutionVenueSnapshot::default();
        let event = MarketEvent {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 1).unwrap(),
            sequence: 2,
            kind: MarketEventKind::Quote {
                bid: 99.9,
                ask: 100.1,
                bid_size: None,
                ask_size: None,
            },
        };
        snapshot.apply_event(&event);
        let intent = ExecutionModel::build_order_intent(
            1,
            SignalAction::GoLong,
            0,
            0,
            None,
            2.0,
            None,
            "event".to_string(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            OrderKind::Market,
        );
        let mut config = execution_config();
        config.market_price_reference = MarketPriceReference::OpposingBest;
        ExecutionModel::record_order_submission(&mut state, &intent);
        model.process_event_pending_order(
            &mut state,
            intent,
            &snapshot,
            &event,
            0,
            &backtest_config(),
            &config,
        );

        let fill = state.fills.first().unwrap();
        assert_eq!(fill.reference_price, 100.1);
        assert_eq!(fill.spread_cost, 0.0);
        assert!((fill.realized_execution_price - 100.16005999999999).abs() < 1e-9);
    }

    #[test]
    fn aggressive_event_limit_fill_is_taker_without_spread_double_count() {
        let mut state = BacktestState {
            cash: 1_000.0,
            ..BacktestState::default()
        };
        let model = ExecutionModel;
        let mut snapshot = ExecutionVenueSnapshot::default();
        let event = MarketEvent {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 1).unwrap(),
            sequence: 2,
            kind: MarketEventKind::Quote {
                bid: 99.9,
                ask: 100.1,
                bid_size: None,
                ask_size: None,
            },
        };
        snapshot.apply_event(&event);
        let intent = ExecutionModel::build_order_intent(
            1,
            SignalAction::GoLong,
            0,
            0,
            None,
            2.0,
            Some(100.2),
            "event_limit".to_string(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            OrderKind::Limit,
        );
        ExecutionModel::record_order_submission(&mut state, &intent);
        model.process_event_pending_order(
            &mut state,
            intent,
            &snapshot,
            &event,
            0,
            &backtest_config(),
            &execution_config(),
        );

        let fill = state.fills.first().unwrap();
        assert!(!fill.maker);
        assert_eq!(fill.spread_cost, 0.0);
        assert_eq!(fill.kind, OrderKind::Limit);
    }

    #[test]
    fn funding_is_applied_at_event_time() {
        let mut state = BacktestState {
            cash: 1_000.0,
            position: Some(Position {
                side: Side::Long,
                quantity: 1.0,
                entry_price: 100.0,
                leverage: 1.0,
                opened_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                margin_allocated: 100.0,
                accumulated_fees: 0.0,
                accumulated_funding: 0.0,
                accumulated_slippage: 0.0,
                accumulated_spread: 0.0,
            }),
            ..BacktestState::default()
        };
        let mut snapshot = ExecutionVenueSnapshot::default();
        snapshot.last_trade_price = Some(100.0);
        ExecutionModel::apply_event_funding(&mut state, &snapshot, 0.001);
        assert!(state.cash < 1_000.0);
        assert_eq!(state.execution_diagnostics.funding_events_applied, 1);
    }

    #[test]
    fn liquidation_uses_event_snapshot_mark() {
        let mut state = BacktestState {
            cash: 10.0,
            position: Some(Position {
                side: Side::Long,
                quantity: 10.0,
                entry_price: 100.0,
                leverage: 10.0,
                opened_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                margin_allocated: 100.0,
                accumulated_fees: 0.0,
                accumulated_funding: 0.0,
                accumulated_slippage: 0.0,
                accumulated_spread: 0.0,
            }),
            ..BacktestState::default()
        };
        let snapshot = ExecutionVenueSnapshot {
            timestamp: Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 1).unwrap()),
            sequence: Some(5),
            best_bid: Some(80.0),
            best_ask: Some(80.2),
            last_trade_price: Some(80.1),
            candle_open: None,
            candle_high: None,
            candle_low: None,
            candle_close: None,
        };
        let mut config = execution_config();
        config.liquidation.maintenance_margin_ratio = 0.2;
        ExecutionModel::maybe_liquidate_snapshot(&mut state, &snapshot, &config);
        assert!(state.position.is_none());
        assert_eq!(state.execution_diagnostics.liquidation_count, 1);
    }

    #[test]
    fn force_flatten_snapshot_rejects_when_no_price_is_available() {
        let mut state = BacktestState {
            cash: 1_000.0,
            position: Some(Position {
                side: Side::Long,
                quantity: 1.0,
                entry_price: 100.0,
                leverage: 1.0,
                opened_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                margin_allocated: 100.0,
                accumulated_fees: 0.0,
                accumulated_funding: 0.0,
                accumulated_slippage: 0.0,
                accumulated_spread: 0.0,
            }),
            ..BacktestState::default()
        };
        let snapshot = ExecutionVenueSnapshot {
            timestamp: Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, 1, 0).unwrap()),
            ..ExecutionVenueSnapshot::default()
        };

        ExecutionModel.force_flatten_snapshot(
            &mut state,
            &snapshot,
            1,
            &backtest_config(),
            &execution_config(),
        );

        assert!(state.position.is_some());
        assert_eq!(state.execution_diagnostics.rejected_orders, 1);
        assert!(matches!(
            state.order_audit_log.last().map(|entry| &entry.status),
            Some(crate::domain::types::OrderStatus::Rejected)
        ));
    }
}
