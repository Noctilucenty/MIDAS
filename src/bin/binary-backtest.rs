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

const USAGE: &str = "usage: binary-backtest <candles.json> <signals.json> \
[--max-concurrent N] [--bankroll AMOUNT] [--payout-prospective] [--full]";

fn run() -> Result<String, String> {
    // Structural parse: walk the argument list once, consuming each option's
    // value explicitly so values never leak into the positional list.
    let mut positional: Vec<String> = Vec::new();
    let mut config = BinaryBacktestConfig::default();
    let mut gates = BinaryPromotionGates::default();
    let mut full = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--max-concurrent" => {
                config.max_concurrent = args
                    .next()
                    .ok_or("--max-concurrent needs a value")?
                    .parse::<usize>()
                    .map_err(|e| format!("bad --max-concurrent: {e}"))?;
                if config.max_concurrent < 1 {
                    return Err("--max-concurrent must be >= 1".to_string());
                }
            }
            "--bankroll" => {
                gates.starting_bankroll = args
                    .next()
                    .ok_or("--bankroll needs a value")?
                    .parse::<f64>()
                    .map_err(|e| format!("bad --bankroll: {e}"))?;
                if gates.starting_bankroll <= 0.0 {
                    return Err("--bankroll must be positive".to_string());
                }
            }
            "--payout-prospective" => gates.payout_source_prospective = true,
            "--full" => full = true,
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other}\n{USAGE}"));
            }
            other => positional.push(other.to_string()),
        }
    }
    if positional.len() != 2 {
        return Err(USAGE.to_string());
    }

    let candles: Vec<Candle> = serde_json::from_str(
        &fs::read_to_string(&positional[0]).map_err(|e| format!("read candles: {e}"))?,
    )
    .map_err(|e| format!("parse candles: {e}"))?;
    let signals: Vec<BinarySignal> = serde_json::from_str(
        &fs::read_to_string(&positional[1]).map_err(|e| format!("read signals: {e}"))?,
    )
    .map_err(|e| format!("parse signals: {e}"))?;

    let mut report =
        run_binary_backtest(&candles, &signals, &config).map_err(|e| e.to_string())?;
    let verdict = evaluate_promotion(&report.metrics, &report.settlements, &gates);
    if !full {
        report.settlements.clear();
    }

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
