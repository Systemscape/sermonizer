use anyhow::{Context, Result, bail};
use serialport::{SerialPortInfo, SerialPortType};
use std::io::{self, Write};

pub fn get_available_ports() -> Result<Vec<SerialPortInfo>> {
    let all_ports = serialport::available_ports().context("Failed to list serial ports")?;

    // Filter for realistic ports (USB ports with VID/PID)
    let ports: Vec<_> = all_ports
        .into_iter()
        .filter(|p| matches!(&p.port_type, SerialPortType::UsbPort(_)))
        .collect();

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
            // Prompt in cooked mode for a clean input experience
            print!("Select port [1-{}] (Enter for 1): ", ports.len());
            let _ = io::stdout().flush();

            // Temporarily disable raw mode if it was on (it isn't yet, but be safe)
            let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
            if was_raw {
                let _ = crossterm::terminal::disable_raw_mode();
            }

            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            if was_raw {
                let _ = crossterm::terminal::enable_raw_mode();
            }

            let sel = line.trim().parse::<usize>().unwrap_or(1);
            let idx = sel.clamp(1, ports.len()) - 1;
            let name = ports[idx].port_name.clone();
            println!("Using port: {name}");
            Ok(name)
        }
    }
}
