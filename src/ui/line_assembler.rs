use chrono::Utc;
use std::fmt::Write as _;

const HEX_BYTES_PER_LINE: usize = 16;
const MAX_TEXT_LINE_BYTES: usize = 4096;

/// Assembles raw serial bytes into display lines. Text mode buffers raw bytes
/// into bounded lines so multi-byte UTF-8 sequences split across reads survive;
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
                if self.partial.len() >= MAX_TEXT_LINE_BYTES
                    && ((b & 0xC0 != 0x80 && b != b'\r')
                        || self.partial.len() >= MAX_TEXT_LINE_BYTES + 4)
                {
                    done.push(self.finish().unwrap());
                    if self.timestamps {
                        self.line_ts = Some(timestamp());
                    }
                }
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

    pub fn finish(&mut self) -> Option<String> {
        let line = if self.hex {
            (!self.hex_row.is_empty()).then(|| std::mem::take(&mut self.hex_row))
        } else if self.partial.is_empty() {
            None
        } else {
            let mut line = self.line_ts.take().unwrap_or_default();
            line.push_str(&String::from_utf8_lossy(&self.partial));
            Some(line)
        };
        self.clear();
        line
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

    #[test]
    fn newline_free_stream_is_bounded_and_preserved() {
        for byte in [b'a', 0x80, b'\r'] {
            let mut asm = LineAssembler::new(false, false);
            let mut lines = Vec::new();
            let chunk = vec![byte; 997];
            for _ in 0..100 {
                lines.extend(asm.push(&chunk));
                assert!(asm.partial.len() <= MAX_TEXT_LINE_BYTES + 4);
            }
            lines.extend(asm.finish());
            assert_eq!(lines.concat(), String::from_utf8_lossy(&vec![byte; 99700]));
        }
    }

    #[test]
    fn long_lines_preserve_utf8_at_each_split_boundary() {
        for offset in 0..4 {
            let text = "a".repeat(MAX_TEXT_LINE_BYTES - offset) + "🦀next\r\n";
            let mut asm = LineAssembler::new(false, false);
            let mut lines = Vec::new();
            for byte in text.bytes() {
                lines.extend(asm.push(&[byte]));
            }
            assert_eq!(lines.concat(), text.trim_end_matches("\r\n"));
            assert_eq!(asm.partial_display(), None);
        }
    }

    #[test]
    fn newline_at_limit_does_not_create_an_extra_line() {
        let mut asm = LineAssembler::new(false, false);
        let text = "a".repeat(MAX_TEXT_LINE_BYTES);
        assert!(asm.push(text.as_bytes()).is_empty());
        assert_eq!(asm.push(b"\r\n"), vec![text]);
    }

    #[test]
    fn finish_preserves_incomplete_utf8_and_resets_timestamps() {
        let mut asm = LineAssembler::new(false, true);
        asm.push(b"before\xF0\x9F");
        assert!(asm.finish().unwrap().ends_with("before�"));
        assert_eq!(asm.finish(), None);
        assert_eq!(asm.line_ts, None);
        assert!(asm.push(b"after\n")[0].ends_with("] after"));
    }

    #[test]
    fn finish_resets_partial_hex_rows() {
        let mut asm = LineAssembler::new(true, false);
        asm.push(&[0xAB; 3]);
        assert_eq!(asm.finish().as_deref(), Some("AB AB AB"));
        assert_eq!(asm.finish(), None);
        assert_eq!(asm.push(&[0xCD; 16]), vec!["CD ".repeat(15) + "CD"]);
    }
}
