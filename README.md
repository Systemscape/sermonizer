# 🔌 Sermonizer

[![CI](https://github.com/USERNAME/sermonizer/workflows/CI/badge.svg)](https://github.com/systemscape/sermonizer/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-brightgreen.svg)](https://www.rust-lang.org)

A simple, clean serial monitor with a clean terminal UI for embedded development.

Most terminal-based serial monitors are annoying to use - they have clunky interfaces and no sane defaults. We wanted something that could be quickly spun up to interact with embedded devices during firmware development without any hassle.

## Features

- **Smart auto-scroll**: Follows new data, easy to switch to manual scrolling
- **Clean TUI**: Split view with input at bottom, output on top
- **Auto-detect ports**: Just run `sermonizer` and it finds your device
- **Sane defaults**: 115200 baud, 8 data bits, no parity, 1 stop bit
- **Hex mode**: View binary data as hex bytes
- **Logging**: Save RX/TX data with timestamps
- **Fast**: Built in Rust, handles high baud rates smoothly

## Quick Start

```bash
# Install
cargo install sermonizer

# Connect to first available port
sermonizer

# Or specify port and baud
sermonizer --port /dev/ttyUSB0 --baud 115200

# List available ports
sermonizer --list
```

## Usage

```bash
sermonizer [OPTIONS]

Options:
  -p, --port <PORT>       Serial port path
  -b, --baud <BAUD>       Baud rate (default: 115200)
      --line-ending <E>   Line ending: none|nl|cr|crlf (default: nl)
      --hex               Display data as hex
      --log <FILE>        Log received data
      --tx-log <FILE>     Log transmitted data
      --log-ts            Add timestamps to logs
      --list              List available ports
```

## Controls

- **Type and press Enter**: Send data to device
- **↑↓ / Page Up/Down**: Scroll through output
- **Ctrl+A**: Re-enable auto-scroll
- **Ctrl+C / Esc**: Exit

## Why?

Perfect for:
- Arduino/ESP32 debugging
- Firmware development workflows
- Quick embedded device interaction
- Protocol testing and development

## License

MIT - see [LICENSE](LICENSE) file.
