use chrono::Local;
use std::fmt::Write as _;

const HEX_BYTES_PER_LINE: usize = 16;
/// Width of a full row of hex bytes ("XX" plus separating spaces)
const HEX_COLS: usize = HEX_BYTES_PER_LINE * 3 - 1;
const MAX_TEXT_LINE_BYTES: usize = 4096;
const ESC: u8 = 0x1B;
const BEL: u8 = 0x07;

/// Position inside an ANSI escape sequence while parsing text mode input.
/// Byte classes follow ECMA-48; anything outside them aborts the sequence so
/// line noise stays visible instead of being swallowed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum EscapeState {
    Text,
    /// After ESC: intermediates (0x20..=0x2F) then a final byte (0x30..=0x7E)
    Escape,
    /// After ESC [: parameters and intermediates (0x20..=0x3F) then a final
    /// byte (0x40..=0x7E)
    Csi,
    /// Inside OSC/DCS/SOS/PM/APC, which run until BEL or ESC \
    Str,
    /// ESC seen inside a string sequence, deciding whether it terminates it
    StrEsc,
}

/// Assembles raw serial bytes into display lines. Text mode buffers raw bytes
/// into bounded lines so multi-byte UTF-8 sequences split across reads survive
/// and drops ANSI escape sequences the list widget cannot render; hex mode
/// emits fixed-width rows.
pub struct LineAssembler {
    hex: bool,
    timestamps: bool,
    strip_ansi: bool,
    escape: EscapeState,
    partial: Vec<u8>,
    hex_ts: String,
    hex_bytes: String,
    hex_ascii: String,
    hex_col: usize,
    line_ts: Option<String>,
}

impl LineAssembler {
    pub fn new(hex: bool, timestamps: bool, strip_ansi: bool) -> Self {
        Self {
            hex,
            timestamps,
            strip_ansi,
            escape: EscapeState::Text,
            partial: Vec::with_capacity(256),
            hex_ts: String::new(),
            hex_bytes: String::new(),
            hex_ascii: String::new(),
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
            if b != b'\n' && self.consume_escape(b) {
                continue;
            }
            // Only displayed bytes start a line, so escape prefixes such as a
            // screen clear do not stamp a line that arrives later
            if self.timestamps && self.line_ts.is_none() {
                self.line_ts = Some(timestamp());
            }
            if b == b'\n' {
                // A newline always ends the line, even inside a broken escape
                self.escape = EscapeState::Text;
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
                    done.push(
                        self.finish()
                            .expect("partial holds at least MAX_TEXT_LINE_BYTES bytes"),
                    );
                    if self.timestamps {
                        self.line_ts = Some(timestamp());
                    }
                }
                self.partial.push(b);
            }
        }
        done
    }

    /// Track ANSI escape sequences; returns true when the byte belongs to one
    /// and must not be displayed.
    fn consume_escape(&mut self, b: u8) -> bool {
        use EscapeState::*;
        if !self.strip_ansi {
            return false;
        }
        let (next, consumed) = match (self.escape, b) {
            (Text, ESC) => (Escape, true),
            (Text, _) => (Text, false),

            (Escape, ESC) => (Escape, true),
            (Escape, b'[') => (Csi, true),
            (Escape, b']' | b'P' | b'X' | b'^' | b'_') => (Str, true),
            (Escape, 0x20..=0x2F) => (Escape, true),
            (Escape, 0x30..=0x7E) => (Text, true),
            (Escape, _) => (Text, false),

            (Csi, ESC) => (Escape, true),
            (Csi, 0x20..=0x3F) => (Csi, true),
            (Csi, 0x40..=0x7E) => (Text, true),
            (Csi, _) => (Text, false),

            (Str, BEL) => (Text, true),
            (Str, ESC) => (StrEsc, true),
            (Str, _) => (Str, true),

            (StrEsc, b'\\') => (Text, true),
            (StrEsc, ESC) => (StrEsc, true),
            (StrEsc, _) => (Str, true),
        };
        self.escape = next;
        consumed
    }

    fn push_hex(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        for &b in bytes {
            if self.hex_col == 0 {
                if self.timestamps {
                    self.hex_ts = timestamp();
                }
            } else {
                self.hex_bytes.push(' ');
            }
            let _ = write!(self.hex_bytes, "{b:02X}");
            self.hex_ascii.push(if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '.'
            });
            self.hex_col += 1;
            if self.hex_col == HEX_BYTES_PER_LINE {
                done.push(self.hex_row());
                self.clear_hex();
            }
        }
        done
    }

    /// Hex row in `hexdump -C` style: bytes, then the printable characters
    fn hex_row(&self) -> String {
        format!(
            "{}{:<HEX_COLS$}  |{}|",
            self.hex_ts, self.hex_bytes, self.hex_ascii
        )
    }

    fn clear_hex(&mut self) {
        self.hex_ts.clear();
        self.hex_bytes.clear();
        self.hex_ascii.clear();
        self.hex_col = 0;
    }

    /// Whether an unfinished line is currently shown below the output.
    pub fn has_partial(&self) -> bool {
        if self.hex {
            self.hex_col > 0
        } else {
            !self.partial.is_empty()
        }
    }

    /// The unfinished line, for display below the completed output.
    pub fn partial_display(&self) -> Option<String> {
        if self.hex {
            (self.hex_col > 0).then(|| self.hex_row())
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
            (self.hex_col > 0).then(|| self.hex_row())
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
        self.escape = EscapeState::Text;
        self.partial.clear();
        self.clear_hex();
        self.line_ts = None;
    }
}

pub fn timestamp() -> String {
    format!("[{}] ", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
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

    fn hex_row(hex: &str, ascii: &str) -> String {
        format!("{hex:<HEX_COLS$}  |{ascii}|")
    }

    #[test]
    fn hex_rows_show_printable_characters_in_a_gutter() {
        let mut asm = LineAssembler::new(true, false, true);
        let rows = asm.push(b"\x01Hi \x7f\xff\x00ok!5678901");
        assert_eq!(
            rows,
            vec![hex_row(
                "01 48 69 20 7F FF 00 6F 6B 21 35 36 37 38 39 30",
                ".Hi ...ok!567890"
            )]
        );
        assert_eq!(asm.partial_display(), Some(hex_row("31", "1")));
    }

    #[test]
    fn text_line_split_across_chunks_completes_once() {
        let mut asm = LineAssembler::new(false, false, true);
        assert!(asm.push(b"hel").is_empty());
        assert_eq!(asm.partial_display().as_deref(), Some("hel"));
        assert_eq!(asm.push(b"lo\nwor"), vec!["hello".to_string()]);
        assert_eq!(asm.partial_display().as_deref(), Some("wor"));
    }

    #[test]
    fn crlf_is_trimmed_from_completed_and_partial_lines() {
        let mut asm = LineAssembler::new(false, false, true);
        assert_eq!(asm.push(b"one\r\ntwo\r"), vec!["one".to_string()]);
        assert_eq!(asm.partial_display().as_deref(), Some("two"));
    }

    #[test]
    fn utf8_sequence_split_across_chunks_stays_intact() {
        let mut asm = LineAssembler::new(false, false, true);
        let bytes = "grün\n".as_bytes();
        assert!(asm.push(&bytes[..3]).is_empty());
        // Incomplete trailing sequence is hidden, not shown as replacement char
        assert_eq!(asm.partial_display().as_deref(), Some("gr"));
        assert_eq!(asm.push(&bytes[3..]), vec!["grün".to_string()]);
        assert_eq!(asm.partial_display(), None);
    }

    #[test]
    fn ansi_escape_sequences_are_stripped_from_text() {
        let mut asm = LineAssembler::new(false, false, true);
        let line = b"\x1b[0;32mI (123) main: ok\x1b[0m\r\n";
        assert_eq!(asm.push(line), vec!["I (123) main: ok".to_string()]);
    }

    #[test]
    fn ansi_escape_split_across_chunks_is_stripped() {
        let mut asm = LineAssembler::new(false, false, true);
        assert!(asm.push(b"a\x1b[").is_empty());
        assert_eq!(asm.partial_display().as_deref(), Some("a"));
        assert_eq!(asm.push(b"1;31mb\n"), vec!["ab".to_string()]);
    }

    #[test]
    fn two_byte_escape_and_newline_inside_escape_are_handled() {
        let mut asm = LineAssembler::new(false, false, true);
        // ESC c (reset) is a two-byte sequence; a newline aborts a broken one
        assert_eq!(
            asm.push(b"\x1bcx\x1b[9\ny\n"),
            vec!["x".to_string(), "y".to_string()]
        );
    }

    #[test]
    fn bytes_outside_csi_ranges_abort_the_sequence_and_stay_visible() {
        let mut asm = LineAssembler::new(false, false, true);
        // Line noise: ESC [ followed by high bytes must not swallow the text
        let out = asm.push(b"good\x1b[\x80\x81 lots of text\n");
        assert_eq!(out, vec!["good\u{FFFD}\u{FFFD} lots of text".to_string()]);
        // ESC followed by a UTF-8 character keeps the character intact
        assert_eq!(
            asm.push("\x1b\u{fc}\n".as_bytes()),
            vec!["\u{fc}".to_string()]
        );
        // Doubled ESC still strips the following SGR
        assert_eq!(asm.push(b"\x1b\x1b[31mred\n"), vec!["red".to_string()]);
    }

    #[test]
    fn string_sequences_are_consumed_up_to_their_terminator() {
        let mut asm = LineAssembler::new(false, false, true);
        assert_eq!(
            asm.push(b"\x1b]0;my board\x07hello\n"),
            vec!["hello".to_string()]
        );
        assert_eq!(
            asm.push(b"\x1b]8;;https://x.io\x1b\\click\x1b]8;;\x1b\\ done\n"),
            vec!["click done".to_string()]
        );
        assert_eq!(
            asm.push(b"\x1bPq#0;2\x1b\\tail\n"),
            vec!["tail".to_string()]
        );
    }

    #[test]
    fn timestamp_is_not_taken_from_escape_bytes() {
        let mut asm = LineAssembler::new(false, true, true);
        assert!(asm.push(b"\x1b[2J").is_empty());
        assert_eq!(asm.line_ts, None);
        assert_eq!(asm.partial_display(), None);
        assert!(asm.push(b"x\n")[0].ends_with("] x"));
    }

    #[test]
    fn raw_mode_keeps_escape_sequences() {
        let mut asm = LineAssembler::new(false, false, false);
        assert_eq!(
            asm.push(b"\x1b[31mred\x1b[0m\n"),
            vec!["\x1b[31mred\x1b[0m".to_string()]
        );
    }

    #[test]
    fn hex_mode_keeps_escape_bytes() {
        let mut asm = LineAssembler::new(true, false, true);
        asm.push(b"\x1b[");
        assert_eq!(asm.partial_display(), Some(hex_row("1B 5B", ".[")));
    }

    #[test]
    fn hex_rows_wrap_at_sixteen_bytes() {
        let mut asm = LineAssembler::new(true, false, true);
        let completed = asm.push(&[0xDE; 18]);
        assert_eq!(
            completed,
            vec![hex_row(&("DE ".repeat(15) + "DE"), &".".repeat(16))]
        );
        assert_eq!(asm.partial_display(), Some(hex_row("DE DE", "..")));
    }

    #[test]
    fn timestamps_prefix_each_completed_line() {
        let mut asm = LineAssembler::new(false, true, true);
        let completed = asm.push(b"a\nb\n");
        assert_eq!(completed.len(), 2);
        for line in &completed {
            assert!(line.starts_with('['), "missing timestamp: {line}");
            assert_eq!(line.matches('[').count(), 1);
        }
    }

    #[test]
    fn clear_resets_partial_state() {
        let mut asm = LineAssembler::new(false, false, true);
        asm.push(b"pending");
        asm.clear();
        assert_eq!(asm.partial_display(), None);
    }

    #[test]
    fn newline_free_stream_is_bounded_and_preserved() {
        for byte in [b'a', 0x80, b'\r'] {
            let mut asm = LineAssembler::new(false, false, true);
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
            let mut asm = LineAssembler::new(false, false, true);
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
        let mut asm = LineAssembler::new(false, false, true);
        let text = "a".repeat(MAX_TEXT_LINE_BYTES);
        assert!(asm.push(text.as_bytes()).is_empty());
        assert_eq!(asm.push(b"\r\n"), vec![text]);
    }

    #[test]
    fn finish_preserves_incomplete_utf8_and_resets_timestamps() {
        let mut asm = LineAssembler::new(false, true, true);
        asm.push(b"before\xF0\x9F");
        assert!(asm.finish().unwrap().ends_with("before�"));
        assert_eq!(asm.finish(), None);
        assert_eq!(asm.line_ts, None);
        assert!(asm.push(b"after\n")[0].ends_with("] after"));
    }

    #[test]
    fn finish_resets_partial_hex_rows() {
        let mut asm = LineAssembler::new(true, false, true);
        asm.push(&[0xAB; 3]);
        assert_eq!(asm.finish(), Some(hex_row("AB AB AB", "...")));
        assert_eq!(asm.finish(), None);
        assert_eq!(
            asm.push(&[0xCD; 16]),
            vec![hex_row(&("CD ".repeat(15) + "CD"), &".".repeat(16))]
        );
    }
}
