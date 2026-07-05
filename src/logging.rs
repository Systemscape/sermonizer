use anyhow::{Context, Result};
use chrono::Utc;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const HEX_BYTES_PER_LINE: usize = 16;

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

/// A log destination that is aware of line boundaries: timestamps are written
/// at the start of each line instead of at arbitrary read-chunk boundaries.
pub struct LogSink<W: Write = BufWriter<File>> {
    writer: W,
    log_ts: bool,
    hex: bool,
    at_line_start: bool,
    hex_col: usize,
}

impl LogSink {
    pub fn open(path: &Path, log_type: &str, log_ts: bool, hex: bool) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("Failed to open {} log file: {}", log_type, path.display()))?;

        println!("Logging {} to: {}", log_type, path.display());
        Ok(Self::new(BufWriter::new(file), log_ts, hex))
    }
}

impl<W: Write> LogSink<W> {
    pub fn new(writer: W, log_ts: bool, hex: bool) -> Self {
        Self {
            writer,
            log_ts,
            hex,
            at_line_start: true,
            hex_col: 0,
        }
    }

    pub fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.hex {
            self.write_hex(bytes)?;
        } else {
            self.write_text(bytes)?;
        }
        self.writer.flush()
    }

    fn write_timestamp(writer: &mut W) -> io::Result<()> {
        write!(writer, "[{}] ", Utc::now().format("%Y-%m-%d %H:%M:%S%.3f"))
    }

    fn write_text(&mut self, bytes: &[u8]) -> io::Result<()> {
        for &b in bytes {
            if self.at_line_start && self.log_ts {
                Self::write_timestamp(&mut self.writer)?;
            }
            self.at_line_start = b == b'\n';
            self.writer.write_all(&[b])?;
        }
        Ok(())
    }

    fn write_hex(&mut self, bytes: &[u8]) -> io::Result<()> {
        for &b in bytes {
            if self.at_line_start {
                if self.log_ts {
                    Self::write_timestamp(&mut self.writer)?;
                }
                self.at_line_start = false;
            } else {
                self.writer.write_all(b" ")?;
            }
            write!(self.writer, "{b:02X}")?;
            self.hex_col += 1;
            if self.hex_col == HEX_BYTES_PER_LINE {
                self.writer.write_all(b"\n")?;
                self.hex_col = 0;
                self.at_line_start = true;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(sink: LogSink<Vec<u8>>) -> String {
        String::from_utf8(sink.writer).expect("log output is valid UTF-8")
    }

    #[test]
    fn text_without_timestamps_passes_bytes_through() {
        let mut sink = LogSink::new(Vec::new(), false, false);
        sink.write_chunk(b"hello ").expect("write");
        sink.write_chunk(b"world\nnext").expect("write");
        assert_eq!(output(sink), "hello world\nnext");
    }

    #[test]
    fn text_timestamps_appear_only_at_line_starts() {
        let mut sink = LogSink::new(Vec::new(), true, false);
        // One line split across two chunks must produce exactly one timestamp
        sink.write_chunk(b"first ").expect("write");
        sink.write_chunk(b"line\nsecond line\n").expect("write");

        let out = output(sink);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(line.starts_with('['), "line missing timestamp: {line}");
            assert_eq!(line.matches('[').count(), 1, "extra timestamp: {line}");
        }
        assert!(lines[0].ends_with("] first line"));
        assert!(lines[1].ends_with("] second line"));
    }

    #[test]
    fn hex_wraps_after_sixteen_bytes() {
        let mut sink = LogSink::new(Vec::new(), false, true);
        sink.write_chunk(&[0xAB; 10]).expect("write");
        sink.write_chunk(&[0xCD; 10]).expect("write");

        let out = output(sink);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            format!(
                "{} {}",
                "AB ".repeat(10).trim_end(),
                "CD ".repeat(6).trim_end()
            )
        );
        assert_eq!(lines[1], "CD CD CD CD");
    }

    #[test]
    fn hex_timestamps_start_each_row() {
        let mut sink = LogSink::new(Vec::new(), true, true);
        sink.write_chunk(&[0x01; 20]).expect("write");

        let out = output(sink);
        for line in out.lines() {
            assert!(line.starts_with('['), "row missing timestamp: {line}");
        }
    }
}
