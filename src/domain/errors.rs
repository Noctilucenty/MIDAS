use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BacktestError {
    #[error("I/O error at {path:?}: {source}")]
    Io {
        path: Option<PathBuf>,
        #[source]
        source: std::io::Error,
    },
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Timestamp parse error for '{value}': {message}")]
    TimestampParse { value: String, message: String },
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("Invalid data: {0}")]
    InvalidData(String),
    #[error("Invariant violation [{name}]: {detail}")]
    InvariantViolation { name: String, detail: String },
    #[error("Unsupported feature: {0}")]
    Unsupported(String),
}

impl BacktestError {
    pub fn io(path: impl Into<Option<PathBuf>>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
