use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::errors::BacktestError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PositionSizing {
    PercentOfEquity { fraction: f64 },
    FixedNotional { notional: f64 },
    FixedQuantity { quantity: f64 },
}

impl Default for PositionSizing {
    fn default() -> Self {
        Self::PercentOfEquity { fraction: 1.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiquidationConfig {
    pub maintenance_margin_ratio: f64,
    pub liquidation_fee_bps: f64,
    #[serde(default = "default_true")]
    pub use_adverse_price_path: bool,
}

impl Default for LiquidationConfig {
    fn default() -> Self {
        Self {
            maintenance_margin_ratio: 0.005,
            liquidation_fee_bps: 10.0,
            use_adverse_price_path: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestConfig {
    pub starting_cash: f64,
    pub default_leverage: f64,
    pub max_leverage: f64,
    #[serde(default)]
    pub position_sizing: PositionSizing,
    #[serde(default = "default_true")]
    pub allow_long: bool,
    #[serde(default = "default_true")]
    pub allow_short: bool,
}

impl BacktestConfig {
    pub fn validate(&self) -> Result<(), BacktestError> {
        if self.starting_cash <= 0.0 {
            return Err(BacktestError::InvalidConfig(
                "starting_cash must be positive".to_string(),
            ));
        }
        if self.default_leverage <= 0.0 || self.max_leverage <= 0.0 {
            return Err(BacktestError::InvalidConfig(
                "leverage must be positive".to_string(),
            ));
        }
        if self.default_leverage > self.max_leverage {
            return Err(BacktestError::InvalidConfig(
                "default_leverage cannot exceed max_leverage".to_string(),
            ));
        }
        match self.position_sizing {
            PositionSizing::PercentOfEquity { fraction } if !(0.0..=1.0).contains(&fraction) => {
                Err(BacktestError::InvalidConfig(
                    "position sizing fraction must be between 0 and 1".to_string(),
                ))
            }
            PositionSizing::FixedNotional { notional } if notional <= 0.0 => Err(
                BacktestError::InvalidConfig("fixed notional must be positive".to_string()),
            ),
            PositionSizing::FixedQuantity { quantity } if quantity <= 0.0 => Err(
                BacktestError::InvalidConfig("fixed quantity must be positive".to_string()),
            ),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrderTypeAssumption {
    Market,
    Limit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketPriceReference {
    OpposingBest,
    Mid,
    LastTrade,
    CandleOpen,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LimitFillAssumption {
    Touch,
    Through,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionConfig {
    pub taker_fee_bps: f64,
    pub maker_fee_bps: f64,
    pub spread_bps: f64,
    pub slippage_bps: f64,
    #[serde(default)]
    pub latency_bars: usize,
    #[serde(default)]
    pub latency_events: usize,
    #[serde(default)]
    pub order_timeout_bars: Option<usize>,
    #[serde(default)]
    pub order_timeout_events: Option<usize>,
    #[serde(default = "default_market_order")]
    pub order_type: OrderTypeAssumption,
    #[serde(default = "default_market_price_reference")]
    pub market_price_reference: MarketPriceReference,
    #[serde(default = "default_limit_fill_assumption")]
    pub limit_fill_assumption: LimitFillAssumption,
    #[serde(default)]
    pub use_candle_spread: bool,
    #[serde(default)]
    pub partial_fill_ratio: Option<f64>,
    #[serde(default)]
    pub liquidation: LiquidationConfig,
}

impl ExecutionConfig {
    pub fn validate(&self) -> Result<(), BacktestError> {
        for (name, value) in [
            ("taker_fee_bps", self.taker_fee_bps),
            ("maker_fee_bps", self.maker_fee_bps),
            ("spread_bps", self.spread_bps),
            ("slippage_bps", self.slippage_bps),
        ] {
            if value < 0.0 {
                return Err(BacktestError::InvalidConfig(format!(
                    "{name} cannot be negative"
                )));
            }
        }
        if let Some(partial_fill_ratio) = self.partial_fill_ratio {
            if !(0.0..=1.0).contains(&partial_fill_ratio) || partial_fill_ratio == 0.0 {
                return Err(BacktestError::InvalidConfig(
                    "partial_fill_ratio must be in (0, 1]".to_string(),
                ));
            }
        }
        if let Some(timeout) = self.order_timeout_bars {
            if timeout == 0 {
                return Err(BacktestError::InvalidConfig(
                    "order_timeout_bars must be at least 1 when provided".to_string(),
                ));
            }
        }
        if let Some(timeout) = self.order_timeout_events {
            if timeout == 0 {
                return Err(BacktestError::InvalidConfig(
                    "order_timeout_events must be at least 1 when provided".to_string(),
                ));
            }
        }
        if self.liquidation.maintenance_margin_ratio < 0.0 {
            return Err(BacktestError::InvalidConfig(
                "maintenance_margin_ratio cannot be negative".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StressScenario {
    pub name: String,
    #[serde(default)]
    pub fee_bps_delta: f64,
    #[serde(default)]
    pub slippage_bps_delta: f64,
    #[serde(default)]
    pub spread_bps_delta: f64,
    #[serde(default)]
    pub latency_bars_delta: usize,
    #[serde(default)]
    pub latency_events_delta: usize,
    #[serde(default = "unit_multiplier")]
    pub funding_multiplier: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ParameterValue {
    Int(i64),
    Float(f64),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterSweep {
    pub name: String,
    pub values: Vec<ParameterValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WalkForwardConfig {
    pub train_ratio: f64,
    pub test_ratio: f64,
    pub step_ratio: f64,
    #[serde(default)]
    pub max_windows: Option<usize>,
}

impl WalkForwardConfig {
    pub fn validate(&self) -> Result<(), BacktestError> {
        for (name, value) in [
            ("train_ratio", self.train_ratio),
            ("test_ratio", self.test_ratio),
            ("step_ratio", self.step_ratio),
        ] {
            if !(0.0..=1.0).contains(&value) || value == 0.0 {
                return Err(BacktestError::InvalidConfig(format!(
                    "{name} must be in (0, 1]"
                )));
            }
        }
        if self.train_ratio + self.test_ratio > 1.0 {
            return Err(BacktestError::InvalidConfig(
                "walk-forward train_ratio + test_ratio cannot exceed 1.0".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegimeWindow {
    pub name: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationConfig {
    pub in_sample_ratio: f64,
    #[serde(default)]
    pub stress_scenarios: Vec<StressScenario>,
    #[serde(default)]
    pub parameter_sweeps: Vec<ParameterSweep>,
    #[serde(default)]
    pub walk_forward: Option<WalkForwardConfig>,
    #[serde(default)]
    pub regime_windows: Vec<RegimeWindow>,
    #[serde(default)]
    pub deterministic_seed: u64,
    #[serde(default = "default_min_trades")]
    pub min_trades_for_score: usize,
}

impl ValidationConfig {
    pub fn validate(&self) -> Result<(), BacktestError> {
        if !(0.1..0.9).contains(&self.in_sample_ratio) {
            return Err(BacktestError::InvalidConfig(
                "in_sample_ratio must be between 0.1 and 0.9".to_string(),
            ));
        }
        for sweep in &self.parameter_sweeps {
            if sweep.values.is_empty() {
                return Err(BacktestError::InvalidConfig(format!(
                    "parameter sweep '{}' must include at least one value",
                    sweep.name
                )));
            }
        }
        if let Some(walk_forward) = &self.walk_forward {
            walk_forward.validate()?;
        }
        for regime in &self.regime_windows {
            if regime.start >= regime.end {
                return Err(BacktestError::InvalidConfig(format!(
                    "regime window '{}' must have start < end",
                    regime.name
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StrategyDefinition {
    MovingAverageCross {
        fast_window: usize,
        slow_window: usize,
    },
}

impl StrategyDefinition {
    pub fn validate(&self) -> Result<(), BacktestError> {
        match self {
            StrategyDefinition::MovingAverageCross {
                fast_window,
                slow_window,
            } => {
                if *fast_window == 0 || *slow_window == 0 {
                    return Err(BacktestError::InvalidConfig(
                        "moving average windows must be positive".to_string(),
                    ));
                }
                if fast_window >= slow_window {
                    return Err(BacktestError::InvalidConfig(
                        "fast_window must be smaller than slow_window".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn apply_parameter(
        &self,
        name: &str,
        value: &ParameterValue,
    ) -> Result<Self, BacktestError> {
        match (self, name, value) {
            (
                StrategyDefinition::MovingAverageCross {
                    fast_window,
                    slow_window,
                },
                "fast_window",
                ParameterValue::Int(new_fast),
            ) if *new_fast > 0 => Ok(StrategyDefinition::MovingAverageCross {
                fast_window: *new_fast as usize,
                slow_window: *slow_window,
            }),
            (
                StrategyDefinition::MovingAverageCross {
                    fast_window,
                    slow_window,
                },
                "slow_window",
                ParameterValue::Int(new_slow),
            ) if *new_slow > 0 => Ok(StrategyDefinition::MovingAverageCross {
                fast_window: *fast_window,
                slow_window: *new_slow as usize,
            }),
            _ => Err(BacktestError::Unsupported(format!(
                "strategy parameter override '{name}' is not supported for this strategy"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunContext {
    pub symbol: String,
    pub venue: Option<String>,
    pub timeframe: String,
    pub run_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestRequest {
    pub context: RunContext,
    pub market_data: crate::domain::types::MarketDataSet,
    pub strategy_input: crate::domain::types::StrategyInput,
    pub backtest_config: BacktestConfig,
    pub execution_config: ExecutionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationRequest {
    pub backtest_request: BacktestRequest,
    pub validation_config: ValidationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunMetadata {
    pub engine_version: String,
    pub artifact_schema_version: String,
    pub run_signature: String,
    pub deterministic_seed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

fn default_market_order() -> OrderTypeAssumption {
    OrderTypeAssumption::Market
}

fn default_market_price_reference() -> MarketPriceReference {
    MarketPriceReference::OpposingBest
}

fn default_limit_fill_assumption() -> LimitFillAssumption {
    LimitFillAssumption::Touch
}

fn unit_multiplier() -> f64 {
    1.0
}

fn default_min_trades() -> usize {
    3
}
