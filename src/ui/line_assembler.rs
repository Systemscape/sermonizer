use chrono::Utc;
use std::fmt::Write as _;

const HEX_BYTES_PER_LINE: usize = 16;

/// Assembles raw serial bytes into display lines. Text mode buffers raw bytes
/// until a newline so multi-byte UTF-8 sequences split across reads survive;
/// hex mode emits fixed-width rows.
pub struct LineAssembler {
    hex: bool,
    timestamps: bool,
    partial: Vec<u8>,
    hex_row: String,
    hex_col: usize,
    line_ts: Option<String>,
}

impl LineAssembler {
    pub fn new(hex: bool, timestamps: bool) -> Self {
        Self {
            hex,
            timestamps,
            partial: Vec::with_capacity(256),
            hex_row: String::new(),
            hex_col: 0,
            line_ts: None,
        }
    }

    /// Feed received bytes; returns any lines completed by this chunk.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        if self.hex {
            self.push_hex(bytes)
        } else {
            self.push_text(bytes)
        }
    }

    fn push_text(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        for &b in bytes {
            if self.timestamps && self.line_ts.is_none() {
                self.line_ts = Some(timestamp());
            }
            if b == b'\n' {
                let mut line = self.line_ts.take().unwrap_or_default();
                let raw = self.partial.strip_suffix(b"\r").unwrap_or(&self.partial);
                line.push_str(&String::from_utf8_lossy(raw));
                done.push(line);
                self.partial.clear();
            } else {
                self.partial.push(b);
            }
        }
        done
    }

    fn push_hex(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        for &b in bytes {
            if self.hex_col == 0 {
                if self.timestamps {
                    self.hex_row.push_str(&timestamp());
                }
            } else {
                self.hex_row.push(' ');
            }
            let _ = write!(self.hex_row, "{b:02X}");
            self.hex_col += 1;
            if self.hex_col == HEX_BYTES_PER_LINE {
                done.push(std::mem::take(&mut self.hex_row));
                self.hex_col = 0;
            }
        }
        done
    }

    /// The unfinished line, for display below the completed output.
    pub fn partial_display(&self) -> Option<String> {
        if self.hex {
            (!self.hex_row.is_empty()).then(|| self.hex_row.clone())
        } else if self.partial.is_empty() {
            None
        } else {
            let mut line = self.line_ts.clone().unwrap_or_default();
            let text = decode_complete_utf8_prefix(&self.partial);
            line.push_str(text.trim_end_matches('\r'));
            Some(line)
        }
    }

    pub fn clear(&mut self) {
        self.partial.clear();
        self.hex_row.clear();
        self.hex_col = 0;
        self.line_ts = None;
    }
}

fn timestamp() -> String {
    format!("[{}] ", Utc::now().format("%Y-%m-%d %H:%M:%S%.3f"))
}

/// Decode the longest UTF-8 prefix, hiding an incomplete trailing sequence
/// until the rest of it arrives.
fn decode_complete_utf8_prefix(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(e) if e.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()
        }
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_line_split_across_chunks_completes_once() {
        let mut asm = LineAssembler::new(false, false);
        assert!(asm.push(b"hel").is_empty());
        assert_eq!(asm.partial_display().as_deref(), Some("hel"));
        assert_eq!(asm.push(b"lo\nwor"), vec!["hello".to_string()]);
        assert_eq!(asm.partial_display().as_deref(), Some("wor"));
    }

    #[test]
    fn crlf_is_trimmed_from_completed_and_partial_lines() {
        let mut asm = LineAssembler::new(false, false);
        assert_eq!(asm.push(b"one\r\ntwo\r"), vec!["one".to_string()]);
        assert_eq!(asm.partial_display().as_deref(), Some("two"));
    }

    #[test]
    fn utf8_sequence_split_across_chunks_stays_intact() {
        let mut asm = LineAssembler::new(false, false);
        let bytes = "grün\n".as_bytes();
        assert!(asm.push(&bytes[..3]).is_empty());
        // Incomplete trailing sequence is hidden, not shown as replacement char
        assert_eq!(asm.partial_display().as_deref(), Some("gr"));
        assert_eq!(asm.push(&bytes[3..]), vec!["grün".to_string()]);
        assert_eq!(asm.partial_display(), None);
    }

    #[test]
    fn hex_rows_wrap_at_sixteen_bytes() {
        let mut asm = LineAssembler::new(true, false);
        let completed = asm.push(&[0xDE; 18]);
        assert_eq!(completed, vec!["DE ".repeat(15) + "DE"]);
        assert_eq!(asm.partial_display().as_deref(), Some("DE DE"));
    }

    #[test]
    fn timestamps_prefix_each_completed_line() {
        let mut asm = LineAssembler::new(false, true);
        let completed = asm.push(b"a\nb\n");
        assert_eq!(completed.len(), 2);
        for line in &completed {
            assert!(line.starts_with('['), "missing timestamp: {line}");
            assert_eq!(line.matches('[').count(), 1);
        }
    }

    #[test]
    fn clear_resets_partial_state() {
        let mut asm = LineAssembler::new(false, false);
        asm.push(b"pending");
        asm.clear();
        assert_eq!(asm.partial_display(), None);
    }
}
