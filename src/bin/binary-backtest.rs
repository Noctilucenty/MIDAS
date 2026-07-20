//! CLI: settle a BinarySignal stream against candles and evaluate promotion gates.
//!
//! Usage:
//!   binary-backtest <candles.json> <signals.json> [--max-concurrent N]
//!                   [--full] (include per-trade settlements in output)
//!
//! candles.json: JSON array of Candle objects (same schema the engine uses).
//! signals.json: JSON array of BinarySignal objects (snake_case serde form,
//! as emitted by the Python train.py orchestrator).
//!
//! Prints a JSON document with the backtest report and the promotion verdict.
//! Exit code 0 = ran successfully (promotable or not); 2 = input/data error.

use std::fs;
use std::process::ExitCode;

use midas_backtesting_engine::domain::binary::BinarySignal;
use midas_backtesting_engine::domain::types::Candle;
use midas_backtesting_engine::engine::binary::{
    run_binary_backtest, BinaryBacktestConfig, BinaryBacktestReport,
};
use midas_backtesting_engine::validation::binary_gates::{
    evaluate_promotion, BinaryPromotionGates, PromotionVerdict,
};

#[derive(serde::Serialize)]
struct Output {
    report: BinaryBacktestReport,
    verdict: PromotionVerdict,
}

fn run() -> Result<String, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if positional.len() != 2 {
        return Err("usage: binary-backtest <candles.json> <signals.json> [--max-concurrent N] [--full]".to_string());
    }

    let mut config = BinaryBacktestConfig::default();
    if let Some(pos) = args.iter().position(|a| a == "--max-concurrent") {
        let value = args
            .get(pos + 1)
            .ok_or("--max-concurrent needs a value")?
            .parse::<usize>()
            .map_err(|e| format!("bad --max-concurrent: {e}"))?;
        config.max_concurrent = value;
    }
    let full = args.iter().any(|a| a == "--full");

    let candles: Vec<Candle> = serde_json::from_str(
        &fs::read_to_string(positional[0]).map_err(|e| format!("read candles: {e}"))?,
    )
    .map_err(|e| format!("parse candles: {e}"))?;
    let signals: Vec<BinarySignal> = serde_json::from_str(
        &fs::read_to_string(positional[1]).map_err(|e| format!("read signals: {e}"))?,
    )
    .map_err(|e| format!("parse signals: {e}"))?;

    let mut report =
        run_binary_backtest(&candles, &signals, &config).map_err(|e| e.to_string())?;
    if !full {
        report.settlements.clear();
    }
    let verdict = evaluate_promotion(&report.metrics, &BinaryPromotionGates::default());

    serde_json::to_string_pretty(&Output { report, verdict }).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    match run() {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}
