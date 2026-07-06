mod config;
mod logging;
mod port_discovery;
mod serial_io;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use config::{
    DataBitsArg, FlowControlArg, LineEnding, ParityArg, PortSettings, StopBitsArg, Toggle, UiConfig,
};
use crossterm::terminal;
use logging::LogSink;
use port_discovery::{choose_port_interactive, get_available_ports, print_ports};
use ratatui::{Terminal, backend::CrosstermBackend};
use serial_io::{SerialEvent, WriterMsg, spawn_supervisor, spawn_writer};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;
use ui::{UiMessage, run_ui};

/// sermonizer — a tiny, friendly serial monitor
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Serial port path/name (auto-detect if omitted)
    #[arg(short, long)]
    port: Option<String>,

    /// Baud rate
    #[arg(short = 'b', long, default_value_t = 115_200)]
    baud: u32,

    /// Line ending when you press Enter (none|nl|cr|crlf). Default: nl
    #[arg(long, value_enum)]
    line_ending: Option<LineEnding>,

    /// Data bits per character
    #[arg(long, value_enum, default_value = "8")]
    data_bits: DataBitsArg,

    /// Parity checking mode
    #[arg(long, value_enum, default_value = "none")]
    parity: ParityArg,

    /// Stop bits
    #[arg(long, value_enum, default_value = "1")]
    stop_bits: StopBitsArg,

    /// Flow control mode
    #[arg(long, value_enum, default_value = "none")]
    flow_control: FlowControlArg,

    /// Set the DTR line after opening (left untouched if omitted)
    #[arg(long, value_enum)]
    dtr: Option<Toggle>,

    /// Set the RTS line after opening (left untouched if omitted)
    #[arg(long, value_enum)]
    rts: Option<Toggle>,

    /// Log received bytes to this file (appends)
    #[arg(long)]
    log: Option<PathBuf>,

    /// Log transmitted bytes to this file (appends)
    #[arg(long)]
    tx_log: Option<PathBuf>,

    /// Prepend timestamps to logged and displayed lines
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
    let baud = args.baud;
    println!("Baud: {baud}");
    println!(
        "Framing: {}{}{}, flow control: {}",
        args.data_bits.label(),
        args.parity.label(),
        args.stop_bits.label(),
        args.flow_control.label()
    );

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
    let settings = PortSettings {
        name: port_name.clone(),
        baud,
        data_bits: args.data_bits.into(),
        parity: args.parity.into(),
        stop_bits: args.stop_bits.into(),
        flow_control: args.flow_control.into(),
        dtr: args.dtr.map(Toggle::as_bool),
        rts: args.rts.map(Toggle::as_bool),
    };
    let port = settings
        .open()
        .with_context(|| format!("Failed to open serial port '{port_name}'"))?;

    println!("Connected. Type to send; press Ctrl-C to exit.\n");

    // Optional log files
    let rx_log = args
        .log
        .as_deref()
        .map(|p| LogSink::open(p, "RX", args.log_ts, args.hex))
        .transpose()?;
    let tx_log = args
        .tx_log
        .as_deref()
        .map(|p| LogSink::open(p, "TX", args.log_ts, false))
        .transpose()?;

    // Handle Ctrl-C with immediate shutdown
    let running = Arc::new(AtomicBool::new(true));
    let shutdown_tx: Arc<StdMutex<Option<mpsc::UnboundedSender<UiMessage>>>> =
        Arc::new(StdMutex::new(None));
    {
        let running = running.clone();
        let shutdown_tx = shutdown_tx.clone();
        ctrlc::set_handler(move || {
            running.store(false, Ordering::SeqCst);
            if let Ok(tx_guard) = shutdown_tx.lock()
                && let Some(tx) = tx_guard.as_ref()
            {
                let _ = tx.send(UiMessage::Quit);
            }
        })
        .context("Failed to set Ctrl-C handler")?;
    }

    // Communication channels for UI
    let (ui_tx, ui_rx) = mpsc::unbounded_channel::<UiMessage>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<SerialEvent>();
    let (writer_tx, writer_rx) = std::sync::mpsc::channel::<WriterMsg>();

    // Store UI sender for Ctrl-C handler
    if let Ok(mut tx_guard) = shutdown_tx.lock() {
        *tx_guard = Some(ui_tx.clone());
    }

    // Reader and writer get independent handles so writes never wait on reads;
    // the supervisor respawns the reader after a disconnect
    let writer_handle = spawn_writer(writer_rx, event_tx.clone(), tx_log);
    let supervisor_handle = spawn_supervisor(
        port,
        settings,
        running.clone(),
        event_tx.clone(),
        writer_tx.clone(),
        rx_log,
    );

    // Setup terminal for ratatui
    terminal::enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let ui_config = UiConfig {
        running: running.clone(),
        line_ending,
        writer: writer_tx.clone(),
        hex: args.hex,
        show_ts: args.log_ts,
        port_label: format!("{port_name} @ {baud}"),
    };

    let ui_res = run_ui(&mut terminal, ui_rx, event_rx, ui_config).await;

    // Cleanup terminal
    terminal::disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Ensure we stop and join the serial threads
    running.store(false, Ordering::SeqCst);
    drop(writer_tx);
    let _ = supervisor_handle.join();
    let _ = writer_handle.join();

    if let Err(e) = ui_res {
        eprintln!("\nError: {e:?}");
    }

    println!("\nDisconnected. Bye!");
    Ok(())
}
