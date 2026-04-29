use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::domain::errors::BacktestError;
use crate::domain::types::MarketEvent;

pub fn load_events_from_json(path: &Path) -> Result<Vec<MarketEvent>, BacktestError> {
    let file =
        File::open(path).map_err(|source| BacktestError::io(Some(path.to_path_buf()), source))?;
    let reader = BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
}
