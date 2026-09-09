mod config;
mod logging;
mod port_discovery;
mod serial_io;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use config::{
    DataBitsArg, FlowControlArg, LineEnding, ParityArg, PortSettings, StopBitsArg, Toggle,
    UiConfig, open_hint, port_label,
};
use logging::LogSink;
use port_discovery::{choose_port_interactive, get_available_ports, print_ports};
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use ratatui::crossterm::execute;
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

    /// Line ending when you press Enter (none|nl|cr|crlf; lf is an alias for nl). Default: nl
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

    /// Prepend timestamps to displayed and logged lines
    #[arg(short = 't', long, alias = "log-ts")]
    timestamps: bool,

    /// Show RX as hex (space-separated bytes)
    #[arg(long)]
    hex: bool,

    /// Keep ANSI escape sequences in the display instead of stripping them
    #[arg(long)]
    raw: bool,

    /// Show what you send in the output, prefixed with "> "
    #[arg(short = 'e', long)]
    echo: bool,

    /// Wrap long lines instead of clipping them (toggle at runtime with Ctrl+T)
    #[arg(short = 'w', long)]
    wrap: bool,

    /// Scroll the output with the mouse wheel (the terminal then needs
    /// Shift+drag to select text)
    #[arg(long)]
    mouse: bool,

    /// Just list ports and exit
    #[arg(long)]
    list: bool,

    /// Also list and offer ports of unknown type, such as onboard UARTs
    /// (/dev/ttyS*), which are hidden by default
    #[arg(long)]
    all_ports: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Enumerate ports up front
    let ports = get_available_ports(args.all_ports)?;

    if args.list {
        // A pager that exits early closes our stdout; that is not an error
        return match print_ports(&mut std::io::stdout().lock(), &ports) {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
                Err(e).context("Failed to print port list")
            }
            _ => Ok(()),
        };
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
    let framing = format!(
        "{}{}{}",
        args.data_bits.label(),
        args.parity.label(),
        args.stop_bits.label()
    );
    println!(
        "Framing: {framing}, flow control: {}",
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
    if args.raw {
        println!("ANSI escapes: kept");
    }
    if args.echo {
        println!("Local echo: ON");
    }
    if args.timestamps {
        println!("Timestamps: ON");
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
    let port = settings.open().map_err(|e| {
        let hint = open_hint(&e).map(|h| format!("\n{h}")).unwrap_or_default();
        anyhow::Error::new(e).context(format!("Failed to open serial port '{port_name}'{hint}"))
    })?;

    println!("Connected. Type to send; press Ctrl-C to exit.\n");

    // Optional log files
    let rx_log = args
        .log
        .as_deref()
        .map(|p| LogSink::open(p, "RX", args.timestamps, args.hex))
        .transpose()?;
    let tx_log = args
        .tx_log
        .as_deref()
        .map(|p| LogSink::open(p, "TX", args.timestamps, false))
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

    // Raw mode + alternate screen, with a panic hook that restores both
    let mut terminal = ratatui::try_init()
        .inspect_err(|_| {
            let _ = ratatui::try_restore();
        })
        .context("Failed to set up terminal")?;
    // Best effort: terminals without bracketed paste still deliver pasted
    // text as key events
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    if args.mouse {
        let _ = execute!(std::io::stdout(), EnableMouseCapture);
    }

    let ui_config = UiConfig {
        running: running.clone(),
        line_ending,
        writer: writer_tx.clone(),
        hex: args.hex,
        show_ts: args.timestamps,
        raw: args.raw,
        echo: args.echo,
        wrap: args.wrap,
        port_label: port_label(&port_name, baud, &framing),
    };

    let ui_res = run_ui(&mut terminal, ui_rx, event_rx, ui_config).await;

    // Restore terminal before anything else can fail
    if args.mouse {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::try_restore().context("Failed to restore terminal")?;
    terminal.show_cursor()?;

    // Ensure we stop and join the serial threads
    running.store(false, Ordering::SeqCst);
    drop(writer_tx);
    let _ = supervisor_handle.join();
    let _ = writer_handle.join();

    if let Err(e) = ui_res {
        eprintln!("\nError: {e:?}");
    }

    println!("\nBye!");
    Ok(())
}
