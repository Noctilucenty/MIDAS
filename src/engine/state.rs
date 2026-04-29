use crate::domain::types::{
    ExecutionDiagnostics, ExecutionLeg, Fill, OrderAuditEntry, Position, ReplayDiagnostics, Trade,
};

#[derive(Debug, Clone)]
pub struct BacktestState {
    pub cash: f64,
    pub position: Option<Position>,
    pub trades: Vec<Trade>,
    pub fills: Vec<Fill>,
    pub order_audit_log: Vec<OrderAuditEntry>,
    pub execution_legs: Vec<ExecutionLeg>,
    pub fee_impact: f64,
    pub funding_impact: f64,
    pub slippage_impact: f64,
    pub spread_impact: f64,
    pub exposed_bars: usize,
    pub next_order_id: u64,
    pub execution_diagnostics: ExecutionDiagnostics,
    pub replay_diagnostics: ReplayDiagnostics,
    pub fill_delay_steps_total: usize,
    pub fill_delay_seconds_total: i64,
}

impl Default for BacktestState {
    fn default() -> Self {
        Self {
            cash: 0.0,
            position: None,
            trades: Vec::new(),
            fills: Vec::new(),
            order_audit_log: Vec::new(),
            execution_legs: Vec::new(),
            fee_impact: 0.0,
            funding_impact: 0.0,
            slippage_impact: 0.0,
            spread_impact: 0.0,
            exposed_bars: 0,
            next_order_id: 1,
            execution_diagnostics: ExecutionDiagnostics::default(),
            replay_diagnostics: ReplayDiagnostics::default(),
            fill_delay_steps_total: 0,
            fill_delay_seconds_total: 0,
        }
    }
}

impl BacktestState {
    pub fn allocate_order_id(&mut self) -> u64 {
        let order_id = self.next_order_id;
        self.next_order_id += 1;
        order_id
    }

    pub fn record_fill_delay(&mut self, steps: usize, seconds: i64) {
        self.fill_delay_steps_total += steps;
        self.fill_delay_seconds_total += seconds;
    }

    pub fn record_reason(&mut self, reason_code: &str) {
        *self
            .execution_diagnostics
            .reason_counts
            .entry(reason_code.to_string())
            .or_insert(0) += 1;
    }

    pub fn finalize_diagnostics(&mut self) {
        if self.execution_diagnostics.filled_orders > 0 {
            self.execution_diagnostics.average_fill_delay_steps = self.fill_delay_steps_total
                as f64
                / self.execution_diagnostics.filled_orders as f64;
            self.execution_diagnostics.average_fill_delay_seconds = self.fill_delay_seconds_total
                as f64
                / self.execution_diagnostics.filled_orders as f64;
        }
        self.execution_diagnostics.total_fee_paid = self.fee_impact;
        self.execution_diagnostics.total_spread_cost = self.spread_impact;
        self.execution_diagnostics.total_slippage_cost = self.slippage_impact;
        self.execution_diagnostics.total_funding_cost = self.funding_impact;
    }
}
