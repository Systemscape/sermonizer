mod config;
mod logging;
mod port_discovery;
mod serial_io;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use config::{LineEnding, UiConfig};
use crossterm::terminal;
use logging::{create_rx_log_writer, create_tx_log_writer};
use port_discovery::{choose_port_interactive, get_available_ports, print_ports};
use ratatui::{backend::CrosstermBackend, Terminal};
use serial_io::{SerialData, SerialReader};
use serialport::SerialPort;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use ui::{run_ui, UiMessage};

/// sermonizer — a tiny, friendly serial monitor
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Serial port path/name (auto-detect if omitted)
    #[arg(short, long)]
    port: Option<String>,

    /// Baud rate (default 115200)
    #[arg(short = 'b', long)]
    baud: Option<u32>,

    /// Line ending when you press Enter (none|nl|cr|crlf). Default: nl
    #[arg(long, value_enum)]
    line_ending: Option<LineEnding>,

    /// Log received bytes to this file (appends)
    #[arg(long)]
    log: Option<PathBuf>,

    /// Log transmitted bytes to this file (appends)
    #[arg(long)]
    tx_log: Option<PathBuf>,

    /// Prepend timestamps to logged chunks (and hex output)
    #[arg(long = "log-ts")]
    log_ts: bool,

    /// Show RX as hex (space-separated bytes)
    #[arg(long)]
    hex: bool,

    /// Just list ports and exit
    #[arg(long)]
    list: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Enumerate ports up front
    let ports = get_available_ports()?;

    if args.list {
        print_ports(&ports);
        return Ok(());
    }

    // Decide on port
    let port_name = match &args.port {
        Some(p) => {
            println!("Using port: {p}");
            p.clone()
        }
        None => choose_port_interactive(&ports)?,
    };

    // Decide on baud
    let baud = match args.baud {
        Some(b) => {
            println!("Baud: {b}");
            b
        }
        None => {
            let b = 115_200u32;
            println!("Baud: {b} (default)");
            b
        }
    };

    // Line ending
    let line_ending = args.line_ending.unwrap_or(LineEnding::Nl);
    if args.line_ending.is_none() {
        println!("Line ending: {} (default)", line_ending.describe());
    } else {
        println!("Line ending: {}", line_ending.describe());
    }

    if args.hex {
        println!("RX view: HEX");
    }
    if args.log_ts {
        println!("Timestamps in logs: ON");
    }

    // Open port
    let mut port = serialport::new(&port_name, baud)
        .timeout(Duration::from_millis(100))
        .open()
        .with_context(|| format!("Failed to open serial port '{port_name}'"))?;

    // Clear any stale data from the serial buffer
    let mut discard_buf = [0u8; 1024];
    while port.read(&mut discard_buf).is_ok() {
        // Keep reading until timeout to flush buffer
    }

    println!("Connected. Type to send; press Ctrl-C to exit.\n");

    // Shared port between reader/writer
    let port: Arc<Mutex<Box<dyn SerialPort + Send>>> = Arc::new(Mutex::new(port));

    // Optional log files
    let rx_log_writer = create_rx_log_writer(args.log.as_ref())?;
    let tx_log_writer = create_tx_log_writer(args.tx_log.as_ref())?;

    // Handle Ctrl-C with immediate shutdown
    let running = Arc::new(AtomicBool::new(true));
    let shutdown_tx: Arc<StdMutex<Option<mpsc::UnboundedSender<UiMessage>>>> =
        Arc::new(StdMutex::new(None));
    {
        let running = running.clone();
        let shutdown_tx = shutdown_tx.clone();
        ctrlc::set_handler(move || {
            running.store(false, Ordering::SeqCst);
            if let Ok(tx_guard) = shutdown_tx.lock() {
                if let Some(tx) = tx_guard.as_ref() {
                    let _ = tx.send(UiMessage::Quit);
                }
            }
        })
        .expect("Failed to set Ctrl-C handler");
    }

    // Communication channels for UI
    let (ui_tx, ui_rx) = mpsc::unbounded_channel::<UiMessage>();
    let (serial_tx, serial_rx) = mpsc::unbounded_channel::<SerialData>();

    // Store UI sender for Ctrl-C handler
    *shutdown_tx.lock().unwrap() = Some(ui_tx.clone());

    // Spawn reader thread (RX) - now using the optimized SerialReader
    let serial_reader = SerialReader::new(
        port.clone(),
        running.clone(),
        serial_tx.clone(),
        args.hex,
        args.log_ts,
        rx_log_writer.clone(),
    );
    let reader_handle = tokio::spawn(async move {
        serial_reader.run().await;
    });

    // Setup terminal for ratatui
    terminal::enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let ui_config = UiConfig {
        running: running.clone(),
        line_ending,
        tx_log: tx_log_writer.clone(),
        log_ts: args.log_ts,
    };

    let ui_res = run_ui(&mut terminal, ui_rx, serial_rx, port.clone(), ui_config).await;

    // Cleanup terminal
    terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Ensure we stop and join reader
    running.store(false, Ordering::SeqCst);
    let _ = reader_handle.await;

    if let Err(e) = ui_res {
        eprintln!("\nError: {e:?}");
    }

    println!("\nDisconnected. Bye!");
    Ok(())
}