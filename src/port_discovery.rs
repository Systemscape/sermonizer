use anyhow::{Context, Result, bail};
use serialport::{SerialPortInfo, SerialPortType};
use std::io::{self, Write};

pub fn get_available_ports() -> Result<Vec<SerialPortInfo>> {
    let mut ports = serialport::available_ports().context("Failed to list serial ports")?;

    // USB ports first: they are the most likely embedded targets, but onboard
    // UARTs, PCI and Bluetooth ports must stay selectable too
    ports.sort_by_key(|p| !matches!(&p.port_type, SerialPortType::UsbPort(_)));

    Ok(ports)
}

pub fn print_ports(ports: &[SerialPortInfo]) {
    if ports.is_empty() {
        println!("No serial ports found.");
        return;
    }
    println!("Available serial ports:");
    for (i, p) in ports.iter().enumerate() {
        print!("  [{}] {}", i + 1, p.port_name);
        match &p.port_type {
            SerialPortType::UsbPort(info) => {
                print!("  (USB");
                print!(" vid=0x{:04x}", info.vid);
                print!(" pid=0x{:04x}", info.pid);
                if let Some(m) = &info.manufacturer {
                    print!(" {m}");
                }
                if let Some(pn) = &info.product {
                    print!(" {pn}");
                }
                print!(")");
            }
            SerialPortType::BluetoothPort => print!("  (Bluetooth)"),
            SerialPortType::PciPort => print!("  (PCI)"),
            SerialPortType::Unknown => {}
        }
        println!();
    }
}

pub fn choose_port_interactive(ports: &[SerialPortInfo]) -> Result<String> {
    match ports.len() {
        0 => bail!("No serial ports detected. Plug your device in and try again."),
        1 => {
            let name = ports[0].port_name.clone();
            println!("Auto-selected sole port: {name}");
            Ok(name)
        }
        _ => {
            print_ports(ports);
            println!();

            // Temporarily disable raw mode if it was on (it isn't yet, but be safe)
            let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
            if was_raw {
                let _ = crossterm::terminal::disable_raw_mode();
            }
            let selection = prompt_for_selection(ports.len());
            if was_raw {
                let _ = crossterm::terminal::enable_raw_mode();
            }

            let name = ports[selection?].port_name.clone();
            println!("Using port: {name}");
            Ok(name)
        }
    }
}

fn prompt_for_selection(count: usize) -> Result<usize> {
    let mut line = String::new();
    loop {
        // Prompt in cooked mode for a clean input experience
        print!("Select port [1-{count}] (Enter for 1, q to quit): ");
        let _ = io::stdout().flush();

        line.clear();
        if io::stdin().read_line(&mut line)? == 0 {
            bail!("No port selected (end of input).");
        }

        let input = line.trim();
        if input.eq_ignore_ascii_case("q") {
            bail!("No port selected.");
        }
        match parse_selection(input, count) {
            Some(idx) => return Ok(idx),
            None => println!("Invalid selection '{input}'. Enter a number between 1 and {count}."),
        }
    }
}

/// Parse a 1-based port selection into a 0-based index. Empty input selects
/// the first port; anything invalid or out of range is rejected.
fn parse_selection(input: &str, count: usize) -> Option<usize> {
    if input.is_empty() {
        return Some(0);
    }
    match input.parse::<usize>() {
        Ok(n) if (1..=count).contains(&n) => Some(n - 1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_selects_first_port() {
        assert_eq!(parse_selection("", 3), Some(0));
    }

    #[test]
    fn valid_numbers_map_to_zero_based_index() {
        assert_eq!(parse_selection("1", 3), Some(0));
        assert_eq!(parse_selection("3", 3), Some(2));
    }

    #[test]
    fn out_of_range_is_rejected() {
        assert_eq!(parse_selection("0", 3), None);
        assert_eq!(parse_selection("4", 3), None);
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(parse_selection("abc", 3), None);
        assert_eq!(parse_selection("-1", 3), None);
        assert_eq!(parse_selection("1.5", 3), None);
    }
}
