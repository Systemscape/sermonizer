use anyhow::{Context, Result, bail};
use serialport::{SerialPortInfo, SerialPortType};
use std::io::{self, Write};

/// Ports worth offering to the user, plus how many were left out.
pub struct PortListing {
    pub shown: Vec<SerialPortInfo>,
    /// Ports of unknown type (typically dead onboard UARTs such as
    /// /dev/ttyS*) that are only listed with --all-ports
    pub hidden: usize,
}

impl PortListing {
    pub fn usb_count(&self) -> usize {
        self.shown.iter().filter(|p| is_usb(p)).count()
    }
}

fn is_usb(port: &SerialPortInfo) -> bool {
    matches!(&port.port_type, SerialPortType::UsbPort(_))
}

pub fn get_available_ports(include_all: bool) -> Result<PortListing> {
    let ports = serialport::available_ports().context("Failed to list serial ports")?;
    Ok(select_ports(ports, include_all))
}

fn select_ports(ports: Vec<SerialPortInfo>, include_all: bool) -> PortListing {
    let total = ports.len();
    let mut shown: Vec<SerialPortInfo> = ports
        .into_iter()
        .filter(|p| include_all || !matches!(p.port_type, SerialPortType::Unknown))
        .collect();
    // USB ports first: they are the most likely embedded targets, but onboard
    // UARTs, PCI and Bluetooth ports must stay selectable too. Within a group
    // ttyUSB2 sorts before ttyUSB10.
    shown.sort_by_cached_key(|p| (!is_usb(p), natural_key(&p.port_name)));
    PortListing {
        hidden: total - shown.len(),
        shown,
    }
}

/// Sort key that orders embedded digit runs numerically
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum NaturalPart {
    Text(String),
    // Comparing length first then digits orders numbers without parsing them
    Number { len: usize, digits: String },
}

fn natural_key(name: &str) -> Vec<NaturalPart> {
    let mut parts = Vec::new();
    let mut run = String::new();
    let mut run_is_digit = false;
    for c in name.chars() {
        if !run.is_empty() && c.is_ascii_digit() != run_is_digit {
            parts.push(natural_part(std::mem::take(&mut run), run_is_digit));
        }
        run_is_digit = c.is_ascii_digit();
        run.push(c);
    }
    if !run.is_empty() {
        parts.push(natural_part(run, run_is_digit));
    }
    parts
}

fn natural_part(run: String, is_digit: bool) -> NaturalPart {
    if is_digit {
        let digits = run.trim_start_matches('0').to_string();
        NaturalPart::Number {
            len: digits.len(),
            digits,
        }
    } else {
        NaturalPart::Text(run)
    }
}

/// Write the port list. Takes a writer instead of printing so a closed pipe
/// (`sermonizer --list | head`) surfaces as an error instead of a panic.
pub fn print_ports(out: &mut impl Write, listing: &PortListing) -> io::Result<()> {
    let ports = &listing.shown;
    if listing.usb_count() == 0 {
        writeln!(out, "No USB serial device found.")?;
    }
    if ports.is_empty() {
        if listing.hidden > 0 {
            writeln!(
                out,
                "{} port(s) of unknown type hidden; use --all-ports to list them.",
                listing.hidden
            )?;
        } else {
            writeln!(out, "No serial ports found.")?;
        }
        return Ok(());
    }
    writeln!(out, "Available serial ports:")?;
    for (i, p) in ports.iter().enumerate() {
        write!(out, "  [{}] {}", i + 1, p.port_name)?;
        match &p.port_type {
            SerialPortType::UsbPort(info) => {
                write!(out, "  (USB vid=0x{:04x} pid=0x{:04x}", info.vid, info.pid)?;
                if let Some(m) = &info.manufacturer {
                    write!(out, " {m}")?;
                }
                if let Some(pn) = &info.product {
                    write!(out, " {pn}")?;
                }
                write!(out, ")")?;
            }
            SerialPortType::BluetoothPort => write!(out, "  (Bluetooth)")?,
            SerialPortType::PciPort => write!(out, "  (PCI)")?,
            SerialPortType::Unknown => {}
        }
        writeln!(out)?;
    }
    if listing.hidden > 0 {
        writeln!(
            out,
            "({} port(s) of unknown type hidden; use --all-ports to list them)",
            listing.hidden
        )?;
    }
    Ok(())
}

pub fn choose_port_interactive(listing: &PortListing) -> Result<String> {
    let ports = &listing.shown;
    // A sole USB port is almost certainly the target device; skip the prompt
    // even when onboard UARTs are also present (USB ports are sorted first)
    if listing.usb_count() == 1 {
        let name = ports[0].port_name.clone();
        println!("Auto-selected sole USB port: {name}");
        return Ok(name);
    }

    match ports.len() {
        0 if listing.hidden > 0 => bail!(
            "No USB serial device found. Plug your device in and try again, \
             or use --all-ports to pick one of the {} port(s) of unknown type.",
            listing.hidden
        ),
        0 => bail!("No serial ports detected. Plug your device in and try again."),
        1 => {
            let name = ports[0].port_name.clone();
            println!("Auto-selected sole port: {name}");
            Ok(name)
        }
        _ => {
            print_ports(&mut io::stdout().lock(), listing)?;
            println!();

            // Temporarily disable raw mode if it was on (it isn't yet, but be safe)
            let was_raw = ratatui::crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
            if was_raw {
                let _ = ratatui::crossterm::terminal::disable_raw_mode();
            }
            let selection = prompt_for_selection(ports.len());
            if was_raw {
                let _ = ratatui::crossterm::terminal::enable_raw_mode();
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
    use serialport::UsbPortInfo;

    fn port(name: &str, port_type: SerialPortType) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.to_string(),
            port_type,
        }
    }

    fn usb() -> SerialPortType {
        SerialPortType::UsbPort(UsbPortInfo {
            vid: 0x10c4,
            pid: 0xea60,
            serial_number: None,
            manufacturer: None,
            product: None,
        })
    }

    fn names(listing: &PortListing) -> Vec<&str> {
        listing.shown.iter().map(|p| p.port_name.as_str()).collect()
    }

    #[test]
    fn unknown_ports_are_hidden_unless_all_requested() {
        let ports = vec![
            port("/dev/ttyS0", SerialPortType::Unknown),
            port("/dev/ttyUSB0", usb()),
            port("/dev/ttyS1", SerialPortType::Unknown),
        ];
        let listing = select_ports(ports.clone(), false);
        assert_eq!(names(&listing), vec!["/dev/ttyUSB0"]);
        assert_eq!(listing.hidden, 2);

        let listing = select_ports(ports, true);
        assert_eq!(
            names(&listing),
            vec!["/dev/ttyUSB0", "/dev/ttyS0", "/dev/ttyS1"]
        );
        assert_eq!(listing.hidden, 0);
    }

    #[test]
    fn ports_sort_usb_first_then_naturally() {
        let ports = vec![
            port("/dev/ttyS10", SerialPortType::Unknown),
            port("/dev/ttyUSB10", usb()),
            port("/dev/ttyS2", SerialPortType::Unknown),
            port("/dev/ttyUSB2", usb()),
            port("/dev/ttyACM0", usb()),
            port("/dev/ttyS1", SerialPortType::PciPort),
        ];
        let listing = select_ports(ports, true);
        assert_eq!(
            names(&listing),
            vec![
                "/dev/ttyACM0",
                "/dev/ttyUSB2",
                "/dev/ttyUSB10",
                "/dev/ttyS1",
                "/dev/ttyS2",
                "/dev/ttyS10"
            ]
        );
    }

    #[test]
    fn print_ports_lists_usb_details_and_hidden_count() {
        let listing = select_ports(
            vec![
                port("/dev/ttyUSB0", usb()),
                port("/dev/ttyS0", SerialPortType::Unknown),
            ],
            false,
        );
        let mut out = Vec::new();
        print_ports(&mut out, &listing).expect("write to a vec");
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "Available serial ports:\n  [1] /dev/ttyUSB0  (USB vid=0x10c4 pid=0xea60)\n\
             (1 port(s) of unknown type hidden; use --all-ports to list them)\n"
        );
    }

    #[test]
    fn print_ports_reports_a_closed_pipe_instead_of_panicking() {
        struct ClosedPipe;
        impl Write for ClosedPipe {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let listing = select_ports(vec![port("/dev/ttyUSB0", usb())], false);
        let err = print_ports(&mut ClosedPipe, &listing).expect_err("pipe is closed");
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn natural_key_orders_com_ports_numerically() {
        let mut names = vec!["COM10", "COM9", "COM1", "COM100"];
        names.sort_by_cached_key(|n| natural_key(n));
        assert_eq!(names, vec!["COM1", "COM9", "COM10", "COM100"]);
    }

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
