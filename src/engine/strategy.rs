use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::domain::config::StrategyDefinition;
use crate::domain::errors::BacktestError;
use crate::domain::types::{Candle, PositionState, Signal, SignalAction};

pub trait Strategy {
    fn on_candle(
        &mut self,
        index: usize,
        candles: &[Candle],
        current_position: PositionState,
    ) -> Option<Signal>;
}

pub fn build_strategy(
    definition: &StrategyDefinition,
) -> Result<Box<dyn Strategy + Send>, BacktestError> {
    match definition {
        StrategyDefinition::MovingAverageCross {
            fast_window,
            slow_window,
        } => Ok(Box::new(MovingAverageCrossStrategy::new(
            *fast_window,
            *slow_window,
        ))),
    }
}

pub struct MovingAverageCrossStrategy {
    fast_window: usize,
    slow_window: usize,
    last_signal_at: Option<DateTime<Utc>>,
}

impl MovingAverageCrossStrategy {
    pub fn new(fast_window: usize, slow_window: usize) -> Self {
        Self {
            fast_window,
            slow_window,
            last_signal_at: None,
        }
    }

    fn sma(candles: &[Candle], end_index: usize, window: usize) -> Option<f64> {
        if end_index + 1 < window {
            return None;
        }
        let start = end_index + 1 - window;
        let sum: f64 = candles[start..=end_index].iter().map(|c| c.close).sum();
        Some(sum / window as f64)
    }
}

impl Strategy for MovingAverageCrossStrategy {
    fn on_candle(
        &mut self,
        index: usize,
        candles: &[Candle],
        current_position: PositionState,
    ) -> Option<Signal> {
        let fast = Self::sma(candles, index, self.fast_window)?;
        let slow = Self::sma(candles, index, self.slow_window)?;
        let timestamp = candles[index].timestamp;
        if self.last_signal_at == Some(timestamp) {
            return None;
        }
        let action = if fast > slow && current_position != PositionState::Long {
            Some(SignalAction::GoLong)
        } else if fast < slow && current_position != PositionState::Short {
            Some(SignalAction::GoShort)
        } else if (fast - slow).abs() < f64::EPSILON && current_position != PositionState::Flat {
            Some(SignalAction::ExitToFlat)
        } else {
            None
        }?;

        self.last_signal_at = Some(timestamp);
        Some(Signal {
            timestamp,
            action,
            leverage_override: None,
            limit_price: None,
            note: Some("moving_average_cross".to_string()),
        })
    }
}

pub struct SignalStreamStrategy {
    signals_by_timestamp: BTreeMap<DateTime<Utc>, Signal>,
}

impl SignalStreamStrategy {
    pub fn new(signals: Vec<Signal>) -> Self {
        let signals_by_timestamp = signals
            .into_iter()
            .map(|signal| (signal.timestamp, signal))
            .collect();
        Self {
            signals_by_timestamp,
        }
    }
}

impl Strategy for SignalStreamStrategy {
    fn on_candle(
        &mut self,
        index: usize,
        candles: &[Candle],
        _current_position: PositionState,
    ) -> Option<Signal> {
        self.signals_by_timestamp
            .get(&candles[index].timestamp)
            .cloned()
    }
}
