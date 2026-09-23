# 🔌 Sermonizer

[![CI](https://github.com/systemscape/sermonizer/actions/workflows/ci.yml/badge.svg)](https://github.com/systemscape/sermonizer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-brightgreen.svg)](https://www.rust-lang.org)

A simple, clean serial monitor with a clean terminal UI for embedded development.

Most terminal-based serial monitors are annoying to use - they have clunky interfaces and no sane defaults. We wanted something that could be quickly spun up to interact with embedded devices during firmware development without any hassle.

```text
┌Serial Monitor────────────────────────────────────────────────────────────────────────────────┐
│[2026-09-10 09:41:02.118] I (312) boot: ESP-IDF v5.2                                          │
│[2026-09-10 09:41:02.121] I (318) wifi: connecting to lab-iot                                 │
│[2026-09-10 09:41:03.877] I (2074) wifi: got ip 192.168.4.23                                  │
│> [2026-09-10 09:41:07.402] AT+GMR                                                            │
│[2026-09-10 09:41:07.410] AT version:2.4.0.0                                                  │
│[2026-09-10 09:41:07.411] OK                                                                  │
│[sermonizer] device disconnected: Broken pipe - reconnecting (Ctrl+C to quit)                 │
│[sermonizer] device reconnected                                                               │
│[2026-09-10 09:41:12.006] I (309) boot: ESP-IDF v5.2                                          │
│[2026-09-10 09:41:12.009] I (315) main: sensor=23.4C hum=41%                                  │
│                                                                                              │
│                                                                                              │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
┌Input─────────────────────────────────────────────────────────────────────────────────────────┐
│AT+CWJAP="lab-iot","                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
 ttyUSB0 115200 8N1  follow | LF | Enter send, Up/Down history, Shift+Up/Down PgUp/PgDn scroll,
```

*Received lines, a sent line (`>`, shown with `--echo`), sermonizer's own notices and the line still being received. Regenerate with `just update-screenshot`.*

## Features

- **Smart auto-scroll**: Follows new data, easy to switch to manual scrolling
- **Auto-reconnect**: Keeps watching the port and resumes when the device comes back
- **Clean TUI**: Split view with input at bottom, output on top, status bar with connection state
- **Auto-detect ports**: Just run `sermonizer` and it finds your device; dead onboard UARTs stay out of the way unless you ask for `--all-ports`
- **Sane defaults**: 115200 baud, 8 data bits, no parity, 1 stop bit
- **Hex mode**: View binary data as hex bytes with a `hexdump -C` style ASCII column
- **Clean text**: ANSI colour codes from firmware logs are stripped (keep them with `--raw`)
- **Local echo**: `--echo` shows what you sent, for devices that do not echo
- **Logging**: Save RX/TX data with timestamps
- **Fast**: Built in Rust, handles high baud rates smoothly

## Quick Start

```bash
# Install from crates.io
cargo install sermonizer

# Install from source
git clone https://github.com/systemscape/sermonizer.git
cd sermonizer
cargo install --path .

# Or run locally
cargo run --release

# Connect to first available port
sermonizer

# Or specify port and baud
sermonizer --port /dev/ttyUSB0 --baud 115200
# Or with cargo run
cargo run --release -- --port /dev/ttyUSB0 --baud 115200

# List available ports
sermonizer --list
# Or with cargo run
cargo run --release -- --list
```

## Usage

```bash
sermonizer [OPTIONS]

Options:
  -p, --port <PORT>        Serial port path
  -b, --baud <BAUD>        Baud rate (default: 115200)
      --line-ending <E>    Line ending: none|nl|lf|cr|crlf (default: nl)
      --data-bits <N>      Data bits: 5|6|7|8 (default: 8)
      --parity <P>         Parity: none|odd|even (default: none)
      --stop-bits <N>      Stop bits: 1|2 (default: 1)
      --flow-control <F>   Flow control: none|software|hardware (default: none)
      --dtr <on|off>       Set the DTR line after opening
      --rts <on|off>       Set the RTS line after opening
      --hex                Display data as hex
      --raw                Keep ANSI escape sequences instead of stripping them
  -e, --echo               Show sent lines in the output, prefixed with "> "
  -w, --wrap               Wrap long lines instead of clipping them
      --mouse              Scroll output with the mouse wheel
      --log <FILE>         Log received data
      --tx-log <FILE>      Log transmitted data
  -t, --timestamps         Add timestamps to display and logs (alias: --log-ts)
      --list               List available ports
      --all-ports          Also list ports of unknown type (e.g. /dev/ttyS*), hidden by default
```

## Controls

- **Type and press Enter**: Send data to device
- **Paste**: Multi-line text is sent line by line; an unfinished last line stays in the input box
- **↑↓**: Browse send history
- **Home / End, Ctrl+A / Ctrl+E**: Jump to start / end of the input line
- **Ctrl+U / Ctrl+K / Ctrl+W**: Delete to start of line / to end of line / previous word
- **Shift+↑↓ / Page Up/Down**: Scroll through output
- **Shift+Home / Shift+End** (or Ctrl+Home / Ctrl+End): Jump to top / bottom of output (End resumes auto-scroll)
- **Mouse wheel**: Scrolls the output when started with `--mouse`. It is off by default because capturing the mouse makes most terminals require Shift+drag to select and copy text
- **Ctrl+T**: Toggle wrapping of long lines
- **Ctrl+L**: Clear output
- **Ctrl+V, then a key**: Send that key as a raw control byte (e.g. Ctrl+V Ctrl+C sends 0x03)
- **Esc**: Clear input line
- **Ctrl+C / Ctrl+D**: Exit

## Why?

Perfect for:
- Arduino/ESP32 debugging
- Firmware development workflows
- Quick embedded device interaction
- Protocol testing and development

## License

MIT - see [LICENSE](LICENSE) file.
