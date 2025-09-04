use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type LogWriter = Arc<Mutex<BufWriter<std::fs::File>>>;

pub fn create_log_writer(path: &PathBuf, log_type: &str) -> Result<LogWriter> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Failed to open {} log file: {}", log_type, path.display()))?;
    
    println!("Logging {} to: {}", log_type, path.display());
    Ok(Arc::new(Mutex::new(BufWriter::new(file))))
}

pub fn create_rx_log_writer(path: Option<&PathBuf>) -> Result<Option<LogWriter>> {
    match path {
        Some(path) => Ok(Some(create_log_writer(path, "RX")?)),
        None => Ok(None),
    }
}

pub fn create_tx_log_writer(path: Option<&PathBuf>) -> Result<Option<LogWriter>> {
    match path {
        Some(path) => Ok(Some(create_log_writer(path, "TX")?)),
        None => Ok(None),
    }
}