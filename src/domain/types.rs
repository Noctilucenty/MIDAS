use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::config::{
    BacktestConfig, ExecutionConfig, RegimeWindow, RunContext, RunMetadata, StrategyDefinition,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub fn sign(self) -> f64 {
        match self {
            Side::Long => 1.0,
            Side::Short => -1.0,
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        }
    }
}

impl From<Side> for PositionState {
    fn from(value: Side) -> Self {
        match value {
            Side::Long => PositionState::Long,
            Side::Short => PositionState::Short,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PositionState {
    Flat,
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    Market,
    Limit,
    Liquidation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Submitted,
    Filled,
    Cancelled,
    Expired,
    Rejected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RobustnessVerdict {
    Passes,
    Borderline,
    Fragile,
    Fails,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLegRole {
    Open,
    Close,
    Liquidation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ValidationReasonCode {
    NegativeOutOfSamplePnl,
    ExcessiveOutOfSampleDrawdown,
    StressFailures,
    ParameterInstability,
    WalkForwardInstability,
    NonDeterministicResults,
    InsufficientTrades,
    InsufficientValidationCoverage,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderBookAction {
    Snapshot,
    Upsert,
    Remove,
    Clear,
}

impl Default for OrderBookAction {
    fn default() -> Self {
        Self::Snapshot
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candle {
    pub timestamp: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    #[serde(default)]
    pub funding_rate: f64,
    #[serde(default)]
    pub spread_bps: Option<f64>,
}

impl Candle {
    pub fn validate(&self) -> bool {
        self.open > 0.0
            && self.high > 0.0
            && self.low > 0.0
            && self.close > 0.0
            && self.high >= self.low
            && self.high >= self.open.min(self.close)
            && self.low <= self.open.max(self.close)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarketEventKind {
    Quote {
        bid: f64,
        ask: f64,
        #[serde(default)]
        bid_size: Option<f64>,
        #[serde(default)]
        ask_size: Option<f64>,
    },
    Bbo {
        bid: f64,
        ask: f64,
        #[serde(default)]
        bid_size: Option<f64>,
        #[serde(default)]
        ask_size: Option<f64>,
    },
    Trade {
        price: f64,
        quantity: f64,
        #[serde(default)]
        aggressor: Option<Side>,
    },
    Funding {
        rate: f64,
    },
    Depth {
        side: Side,
        price: f64,
        quantity: f64,
        #[serde(default)]
        level: Option<u32>,
        #[serde(default)]
        action: OrderBookAction,
    },
    Candle {
        candle: Candle,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketEvent {
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub sequence: u64,
    #[serde(flatten)]
    pub kind: MarketEventKind,
}

impl MarketEvent {
    pub fn validate(&self) -> bool {
        match &self.kind {
            MarketEventKind::Quote { bid, ask, .. } | MarketEventKind::Bbo { bid, ask, .. } => {
                *bid > 0.0 && *ask > 0.0 && ask >= bid
            }
            MarketEventKind::Trade {
                price, quantity, ..
            } => *price > 0.0 && *quantity > 0.0,
            MarketEventKind::Funding { .. } => true,
            MarketEventKind::Depth {
                price, quantity, ..
            } => *price > 0.0 && *quantity >= 0.0,
            MarketEventKind::Candle { candle } => candle.validate(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", content = "data", rename_all = "snake_case")]
pub enum MarketDataSet {
    Candles(Vec<Candle>),
    Events(Vec<MarketEvent>),
}

impl MarketDataSet {
    pub fn candles(&self) -> &[Candle] {
        match self {
            MarketDataSet::Candles(candles) => candles,
            MarketDataSet::Events(_) => &[],
        }
    }

    pub fn events(&self) -> &[MarketEvent] {
        match self {
            MarketDataSet::Events(events) => events,
            MarketDataSet::Candles(_) => &[],
        }
    }

    pub fn len(&self) -> usize {
        match self {
            MarketDataSet::Candles(candles) => candles.len(),
            MarketDataSet::Events(events) => events.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn mode_label(&self) -> &'static str {
        match self {
            MarketDataSet::Candles(_) => "candles",
            MarketDataSet::Events(_) => "events",
        }
    }

    pub fn time_bounds(&self) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
        match self {
            MarketDataSet::Candles(candles) => {
                Some((candles.first()?.timestamp, candles.last()?.timestamp))
            }
            MarketDataSet::Events(events) => {
                Some((events.first()?.timestamp, events.last()?.timestamp))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SignalAction {
    GoLong,
    GoShort,
    ExitToFlat,
    Hold,
}

impl SignalAction {
    pub fn target_side(&self) -> Option<Side> {
        match self {
            SignalAction::GoLong => Some(Side::Long),
            SignalAction::GoShort => Some(Side::Short),
            SignalAction::ExitToFlat | SignalAction::Hold => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Signal {
    pub timestamp: DateTime<Utc>,
    pub action: SignalAction,
    #[serde(default)]
    pub leverage_override: Option<f64>,
    #[serde(default)]
    pub limit_price: Option<f64>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum StrategyInput {
    Definition(StrategyDefinition),
    SignalStream(Vec<Signal>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderIntent {
    pub order_id: u64,
    pub generated_at: DateTime<Utc>,
    pub generated_index: usize,
    pub execute_at_index: usize,
    pub expires_at_index: Option<usize>,
    pub action: SignalAction,
    pub requested_kind: OrderKind,
    pub leverage: f64,
    pub limit_price: Option<f64>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fill {
    pub order_id: u64,
    pub submitted_at: DateTime<Utc>,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub trigger_sequence: Option<u64>,
    pub kind: OrderKind,
    pub side: Side,
    pub quantity: f64,
    pub reference_price: f64,
    pub realized_execution_price: f64,
    pub fee_paid: f64,
    pub spread_cost: f64,
    pub slippage_cost: f64,
    pub liquidation: bool,
    pub maker: bool,
    pub fill_delay_steps: usize,
    pub fill_delay_seconds: i64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderAuditEntry {
    pub order_id: u64,
    pub generated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub trigger_sequence: Option<u64>,
    pub action: SignalAction,
    pub requested_kind: OrderKind,
    pub realized_kind: Option<OrderKind>,
    pub status: OrderStatus,
    pub limit_price: Option<f64>,
    pub fill_price: Option<f64>,
    pub quantity: Option<f64>,
    pub maker: Option<bool>,
    pub fill_delay_steps: Option<usize>,
    pub fill_delay_seconds: Option<i64>,
    pub execution_leg_count: Option<usize>,
    pub position_before: Option<PositionState>,
    pub position_after: Option<PositionState>,
    pub reason_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionLeg {
    pub order_id: u64,
    pub leg_index: usize,
    pub role: ExecutionLegRole,
    pub generated_at: DateTime<Utc>,
    pub executed_at: DateTime<Utc>,
    #[serde(default)]
    pub trigger_sequence: Option<u64>,
    pub requested_kind: OrderKind,
    pub realized_kind: OrderKind,
    pub side: Side,
    pub quantity: f64,
    pub reference_price: f64,
    pub realized_execution_price: f64,
    pub maker: bool,
    pub fee_paid: f64,
    pub spread_cost: f64,
    pub slippage_cost: f64,
    pub funding_cost: f64,
    #[serde(default)]
    pub gross_pnl: Option<f64>,
    #[serde(default)]
    pub net_pnl: Option<f64>,
    pub fill_delay_steps: usize,
    pub fill_delay_seconds: i64,
    pub position_before: PositionState,
    pub position_after: PositionState,
    pub liquidation: bool,
    pub reason_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Position {
    pub side: Side,
    pub quantity: f64,
    pub entry_price: f64,
    pub leverage: f64,
    pub opened_at: DateTime<Utc>,
    pub margin_allocated: f64,
    pub accumulated_fees: f64,
    pub accumulated_funding: f64,
    pub accumulated_slippage: f64,
    pub accumulated_spread: f64,
}

impl Position {
    pub fn state(&self) -> PositionState {
        match self.side {
            Side::Long => PositionState::Long,
            Side::Short => PositionState::Short,
        }
    }

    pub fn signed_quantity(&self) -> f64 {
        self.quantity * self.side.sign()
    }

    pub fn notional(&self, mark_price: f64) -> f64 {
        self.quantity.abs() * mark_price
    }

    pub fn unrealized_pnl(&self, mark_price: f64) -> f64 {
        self.quantity * (mark_price - self.entry_price) * self.side.sign()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Trade {
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
    pub side: Side,
    pub quantity: f64,
    pub leverage: f64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub gross_pnl: f64,
    pub net_pnl: f64,
    pub fees_paid: f64,
    pub funding_paid: f64,
    pub slippage_paid: f64,
    pub spread_paid: f64,
    pub duration_seconds: i64,
    pub liquidated: bool,
    pub entry_note: Option<String>,
    pub exit_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EquityPoint {
    pub timestamp: DateTime<Utc>,
    pub equity: f64,
    pub cash: f64,
    pub unrealized_pnl: f64,
    pub position_state: PositionState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DrawdownPoint {
    pub timestamp: DateTime<Utc>,
    pub equity: f64,
    pub peak_equity: f64,
    pub drawdown: f64,
    pub drawdown_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ReplayDiagnostics {
    pub mode: String,
    pub processed_steps: usize,
    pub processed_candles: usize,
    pub processed_events: usize,
    pub quote_events: usize,
    pub bbo_events: usize,
    pub trade_events: usize,
    pub funding_events: usize,
    pub depth_events: usize,
    pub candle_events: usize,
    pub reordered_events: usize,
    pub last_sequence: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ExecutionDiagnostics {
    pub submitted_orders: usize,
    pub filled_orders: usize,
    pub cancelled_orders: usize,
    pub expired_orders: usize,
    pub rejected_orders: usize,
    pub market_fills: usize,
    pub maker_limit_fills: usize,
    pub taker_limit_fills: usize,
    pub open_legs: usize,
    pub close_legs: usize,
    pub liquidation_legs: usize,
    pub flip_orders: usize,
    pub liquidation_count: usize,
    pub funding_events_applied: usize,
    pub average_fill_delay_steps: f64,
    pub average_fill_delay_seconds: f64,
    pub total_fee_paid: f64,
    pub total_spread_cost: f64,
    pub total_slippage_cost: f64,
    pub total_funding_cost: f64,
    pub total_liquidation_fees: f64,
    #[serde(default)]
    pub reason_counts: BTreeMap<String, usize>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsistencyCheckResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ConsistencyReport {
    pub checks: Vec<ConsistencyCheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsReport {
    pub total_return: f64,
    pub net_pnl: f64,
    pub gross_pnl: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub average_win: f64,
    pub average_loss: f64,
    pub trade_count: usize,
    pub exposure_time_pct: f64,
    pub average_trade_duration_seconds: f64,
    pub funding_cost_impact: f64,
    pub fee_impact: f64,
    pub slippage_impact: f64,
    pub spread_impact: f64,
    pub ending_equity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunManifest {
    pub engine_version: String,
    pub artifact_schema_version: String,
    pub request_signature: String,
    pub data_mode: String,
    pub dataset_length: usize,
    pub range_start: Option<DateTime<Utc>>,
    pub range_end: Option<DateTime<Utc>>,
    pub input_summary: BTreeMap<String, String>,
    pub assumption_summary: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestArtifacts {
    pub equity_curve: Vec<EquityPoint>,
    pub drawdown_curve: Vec<DrawdownPoint>,
    pub trade_log: Vec<Trade>,
    pub fills: Vec<Fill>,
    pub order_audit_log: Vec<OrderAuditEntry>,
    pub execution_legs: Vec<ExecutionLeg>,
    pub execution_diagnostics: ExecutionDiagnostics,
    pub replay_diagnostics: ReplayDiagnostics,
    pub consistency_report: ConsistencyReport,
    pub assumptions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestReport {
    pub metadata: RunMetadata,
    pub manifest: RunManifest,
    pub context: RunContext,
    pub backtest_config: BacktestConfig,
    pub execution_config: ExecutionConfig,
    pub strategy_input: StrategyInput,
    pub metrics: MetricsReport,
    pub artifacts: BacktestArtifacts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitReport {
    pub label: String,
    pub range_start: DateTime<Utc>,
    pub range_end: DateTime<Utc>,
    pub metrics: MetricsReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StressTestReport {
    pub scenario_name: String,
    pub metrics: MetricsReport,
    pub net_pnl_delta_from_base: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterSweepResult {
    pub parameter_values: BTreeMap<String, String>,
    pub metrics: MetricsReport,
    pub net_pnl_delta_from_base: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WalkForwardWindowReport {
    pub window_index: usize,
    pub training_range_start: DateTime<Utc>,
    pub training_range_end: DateTime<Utc>,
    pub test_range_start: DateTime<Utc>,
    pub test_range_end: DateTime<Utc>,
    pub training_metrics: MetricsReport,
    pub test_metrics: MetricsReport,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegimeReport {
    pub regime: RegimeWindow,
    pub metrics: MetricsReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationScoreBreakdown {
    pub profitability: f64,
    pub drawdown_control: f64,
    pub stability: f64,
    pub sensitivity: f64,
    pub stress_survivability: f64,
    pub determinism: f64,
    pub trade_sufficiency: f64,
    pub total: f64,
    pub max_total: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationDiagnostics {
    pub degradation_ratio: f64,
    pub walk_forward_pass_rate: f64,
    pub worst_stress_net_pnl: Option<f64>,
    pub best_parameter_net_pnl: Option<f64>,
    pub worst_parameter_net_pnl: Option<f64>,
    pub score_explanation: Vec<String>,
    #[serde(default)]
    pub verdict_explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationSummary {
    pub score: f64,
    pub passed: bool,
    pub verdict: RobustnessVerdict,
    pub deterministic_reproducible: bool,
    pub stress_pass_rate: f64,
    pub sensitivity_pass_rate: f64,
    pub walk_forward_pass_rate: f64,
    #[serde(default)]
    pub reason_codes: Vec<ValidationReasonCode>,
    #[serde(default)]
    pub primary_reason: Option<ValidationReasonCode>,
    pub breakdown: ValidationScoreBreakdown,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationReport {
    pub metadata: RunMetadata,
    pub base_report: BacktestReport,
    pub in_sample: SplitReport,
    pub out_of_sample: SplitReport,
    pub stress_tests: Vec<StressTestReport>,
    pub parameter_sensitivity: Vec<ParameterSweepResult>,
    pub walk_forward: Vec<WalkForwardWindowReport>,
    pub regime_reports: Vec<RegimeReport>,
    pub diagnostics: ValidationDiagnostics,
    pub summary: ValidationSummary,
}
