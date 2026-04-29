use std::path::PathBuf;

use clap::{Parser, Subcommand};
use midas_backtesting_engine::api::service::{
    BacktestService, FileBacktestRequest, FileValidationRequest,
};
use midas_backtesting_engine::domain::errors::BacktestError;
use midas_backtesting_engine::reporting::{write_backtest_report, write_validation_report};

#[derive(Parser, Debug)]
#[command(
    name = "midas-cli",
    about = "Deterministic crypto strategy backtesting and validation CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Backtest {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Validate {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

fn main() -> Result<(), BacktestError> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Backtest { config, output } => {
            let file_request: FileBacktestRequest = BacktestService::load_json(&config)?;
            let report = BacktestService::run_backtest_from_file_request(file_request)?;
            if let Some(path) = output {
                write_backtest_report(&path, &report)?;
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Commands::Validate { config, output } => {
            let file_request: FileValidationRequest = BacktestService::load_json(&config)?;
            let report = BacktestService::run_validation_from_file_request(file_request)?;
            if let Some(path) = output {
                write_validation_report(&path, &report)?;
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}
