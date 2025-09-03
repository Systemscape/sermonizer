use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal,
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use serialport::{SerialPort, SerialPortType};
use std::fs::OpenOptions;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Mutex, mpsc};
use std::time::Duration;

/// Which line ending to send when you press Enter
#[derive(Copy, Clone, Debug, ValueEnum)]
enum LineEnding {
    /// Send nothing extra (no line ending)
    None,
    /// Send '\n' (LF)
    Nl,
    /// Send '\r' (CR)
    Cr,
    /// Send "\r\n" (CRLF)
    Crlf,
}

impl LineEnding {
    fn describe(self) -> &'static str {
        match self {
            LineEnding::None => "none",
            LineEnding::Nl => "LF (\\n)",
            LineEnding::Cr => "CR (\\r)",
            LineEnding::Crlf => "CRLF (\\r\\n)",
        }
    }
    fn bytes(self) -> &'static [u8] {
        match self {
            LineEnding::None => b"",
            LineEnding::Nl => b"\n",
            LineEnding::Cr => b"\r",
            LineEnding::Crlf => b"\r\n",
        }
    }
}

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

fn now_rfc3339() -> String {
    // Simple RFC3339-ish time without timezone math (system local time)
    let now = chrono_like_now();
    format!("{}", now)
}

// Minimal, std-only timestamp (YYYY-MM-DD HH:MM:SS.mmm)
fn chrono_like_now() -> impl std::fmt::Display {
    use std::fmt;
    use std::time::SystemTime;
    struct Stamp(u128);
    impl fmt::Display for Stamp {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let ms = self.0;
            let secs = (ms / 1000) as i64;
            let millis = (ms % 1000) as u32;
            // Best effort human time; avoid external deps
            let tm = time_conv(secs);
            write!(
                f,
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
                tm.0, tm.1, tm.2, tm.3, tm.4, tm.5, millis
            )
        }
    }
    fn time_conv(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
        // Very small local converter: assume UTC for portability.
        // If you prefer local time, swap this for chrono.
        let tm = secs_to_ymdhms_utc(secs);
        (tm.0, tm.1, tm.2, tm.3, tm.4, tm.5)
    }
    fn secs_to_ymdhms_utc(s: i64) -> (i32, u32, u32, u32, u32, u32) {
        // Algorithm adapted from civil time conversions; fine for logs.
        const SECS_PER_DAY: i64 = 86_400;
        let z = s.div_euclid(SECS_PER_DAY);
        let secs_of_day = s.rem_euclid(SECS_PER_DAY);
        let a = z + 719468;
        let era = if a >= 0 { a } else { a - 146096 };
        let doe = a - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = (yoe as i32) + era as i32 * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = mp + if mp < 10 { 3 } else { -9 };
        let y = y + (m <= 2) as i32;
        let hour = (secs_of_day / 3600) as u32;
        let min = ((secs_of_day % 3600) / 60) as u32;
        let sec = (secs_of_day % 60) as u32;
        (y, m as u32, d as u32, hour, min, sec)
    }
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis();
    Stamp(ms)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Enumerate ports up front
    let all_ports = serialport::available_ports().context("Failed to list serial ports")?;

    // Filter for realistic ports (USB ports with VID/PID)
    let ports: Vec<_> = all_ports
        .into_iter()
        .filter(|p| matches!(&p.port_type, SerialPortType::UsbPort(_)))
        .collect();

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
    let rx_log_writer: Option<Arc<StdMutex<BufWriter<std::fs::File>>>> = match &args.log {
        Some(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("Failed to open log file: {}", path.display()))?;
            println!("Logging RX to: {}", path.display());
            Some(Arc::new(StdMutex::new(BufWriter::new(file))))
        }
        None => None,
    };
    let tx_log_writer: Option<Arc<StdMutex<BufWriter<std::fs::File>>>> = match &args.tx_log {
        Some(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("Failed to open tx-log file: {}", path.display()))?;
            println!("Logging TX to: {}", path.display());
            Some(Arc::new(StdMutex::new(BufWriter::new(file))))
        }
        None => None,
    };

    // Handle Ctrl-C with immediate shutdown
    let running = Arc::new(AtomicBool::new(true));
    let shutdown_tx: Arc<StdMutex<Option<mpsc::UnboundedSender<UiMessage>>>> = Arc::new(StdMutex::new(None));
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
        .expect("Failed to set Ctrl-C handler");
    }

    // Communication channels for UI
    let (ui_tx, ui_rx) = mpsc::unbounded_channel::<UiMessage>();
    let (serial_tx, serial_rx) = mpsc::unbounded_channel::<SerialData>();

    // Store UI sender for Ctrl-C handler
    *shutdown_tx.lock().unwrap() = Some(ui_tx.clone());

    // Spawn reader thread (RX)
    let port_reader = port.clone();
    let running_reader = running.clone();
    let rx_log_writer_reader = rx_log_writer.clone();
    let hex_mode = args.hex;
    let log_ts = args.log_ts;
    let serial_tx_clone = serial_tx.clone();
    let reader_handle = tokio::spawn(async move {
        let mut buf = [0u8; 4096];

        while running_reader.load(Ordering::SeqCst) {
            let n = {
                let mut guard = port_reader.lock().await;
                match guard.read(&mut buf) {
                    Ok(n) => n,
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => 0,
                    Err(_) => break,
                }
            };

            if n > 0 {
                let bytes = &buf[..n];

                // Format the data
                let display_text = if hex_mode {
                    let mut hex_str = String::new();
                    if log_ts {
                        hex_str.push_str(&format!("[{}] ", now_rfc3339()));
                    }
                    for (i, b) in bytes.iter().enumerate() {
                        hex_str.push_str(&format!(
                            "{:02X}{}",
                            b,
                            if i + 1 == bytes.len() { "" } else { " " }
                        ));
                    }
                    hex_str
                } else {
                    let mut text = String::new();
                    if log_ts {
                        text.push_str(&format!("[{}] ", now_rfc3339()));
                    }
                    text.push_str(&String::from_utf8_lossy(bytes));
                    text
                };

                // Send to UI
                let _ = serial_tx_clone.send(SerialData::Received(display_text));

                // RX log file
                if let Some(w) = &rx_log_writer_reader {
                    if let Ok(mut lw) = w.lock() {
                        if log_ts {
                            let _ = write!(lw, "[{}] ", now_rfc3339());
                        }
                        if hex_mode {
                            for (i, b) in bytes.iter().enumerate() {
                                let _ = write!(
                                    lw,
                                    "{:02X}{}",
                                    b,
                                    if i + 1 == bytes.len() { "" } else { " " }
                                );
                            }
                            let _ = writeln!(lw);
                        } else {
                            let _ = lw.write_all(bytes);
                        }
                        let _ = lw.flush();
                    }
                }
            } else {
                // Small async yield to prevent busy waiting
                tokio::task::yield_now().await;
            }
        }
    });

    // Setup terminal for ratatui
    terminal::enable_raw_mode().context("Failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let ui_res = run_ui(
        &mut terminal,
        ui_rx,
        serial_rx,
        SerialConfig { port: port.clone() },
        UiConfig {
            running: running.clone(),
            line_ending,
            tx_log: tx_log_writer.clone(),
            log_ts: args.log_ts,
        },
    ).await;

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

fn print_ports(ports: &[serialport::SerialPortInfo]) {
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

fn choose_port_interactive(ports: &[serialport::SerialPortInfo]) -> Result<String> {
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
            let _ = std::io::stdout().flush();

            // Temporarily disable raw mode if it was on (it isn’t yet, but be safe)
            let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
            if was_raw {
                let _ = crossterm::terminal::disable_raw_mode();
            }

            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
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

#[derive(Debug)]
enum UiMessage {
    Quit,
}

#[derive(Debug, Clone)]
enum SerialData {
    Received(String),
}

struct AppState {
    input_line: String,
    output_lines: Vec<String>,
    partial_line: String,
    list_state: ListState,
    auto_scroll_state: ListState,
    should_quit: bool,
    auto_scroll: bool,
}

impl AppState {
    fn new() -> Self {
        Self {
            input_line: String::new(),
            output_lines: Vec::new(),
            partial_line: String::new(),
            list_state: ListState::default(),
            auto_scroll_state: ListState::default(),
            should_quit: false,
            auto_scroll: true,
        }
    }

    fn add_output(&mut self, data: String) {
        // Append to partial line buffer
        self.partial_line.push_str(&data);
        
        // Check if we have complete lines (ending with \n or \r\n)
        while let Some(newline_pos) = self.partial_line.find('\n') {
            // Extract complete line (without the newline)
            let complete_line = self.partial_line[..newline_pos].trim_end_matches('\r').to_string();
            self.output_lines.push(complete_line);
            
            // Remove processed part from partial_line
            self.partial_line.drain(..=newline_pos);
        }

        // Keep only the last 1000 lines to prevent memory issues
        if self.output_lines.len() > 1000 {
            self.output_lines.drain(..self.output_lines.len() - 1000);
        }

        // Update auto-scroll state to point to the new bottom
        if !self.output_lines.is_empty() {
            self.auto_scroll_state
                .select(Some(self.output_lines.len() - 1));
        }
    }

    fn scroll_up(&mut self) {
        if self.output_lines.is_empty() {
            return;
        }
        // Disable auto-scroll when manually scrolling
        self.auto_scroll = false;

        let selected = self
            .list_state
            .selected()
            .unwrap_or(self.output_lines.len() - 1);
        if selected > 0 {
            self.list_state.select(Some(selected - 1));
        }
    }

    fn scroll_down(&mut self) {
        if self.output_lines.is_empty() {
            return;
        }
        // Disable auto-scroll when manually scrolling
        self.auto_scroll = false;

        let selected = self.list_state.selected().unwrap_or(0);
        if selected < self.output_lines.len() - 1 {
            self.list_state.select(Some(selected + 1));
        }
    }

    fn scroll_to_bottom(&mut self) {
        if !self.output_lines.is_empty() {
            // Disable auto-scroll when manually scrolling to bottom
            self.auto_scroll = false;
            self.list_state.select(Some(self.output_lines.len() - 1));
        }
    }

    fn enable_auto_scroll(&mut self) {
        self.auto_scroll = true;
        self.list_state.select(None); // Clear selection when re-enabling auto-scroll
    }

    fn scroll_to_home(&mut self) {
        if !self.output_lines.is_empty() {
            // Disable auto-scroll when manually scrolling to top
            self.auto_scroll = false;
            self.list_state.select(Some(0));
        }
    }
}

struct SerialConfig {
    port: Arc<Mutex<Box<dyn SerialPort + Send>>>,
}

struct UiConfig {
    running: Arc<AtomicBool>,
    line_ending: LineEnding,
    tx_log: Option<Arc<StdMutex<BufWriter<std::fs::File>>>>,
    log_ts: bool,
}

async fn run_ui<B: Backend>(
    terminal: &mut Terminal<B>,
    mut ui_rx: mpsc::UnboundedReceiver<UiMessage>,
    mut serial_rx: mpsc::UnboundedReceiver<SerialData>,
    serial_config: SerialConfig,
    ui_config: UiConfig,
) -> Result<()> {
    let mut app_state = AppState::new();

    while ui_config.running.load(Ordering::SeqCst) && !app_state.should_quit {
        // Check for UI messages (like quit from Ctrl-C)
        if let Ok(msg) = ui_rx.try_recv() {
            match msg {
                UiMessage::Quit => {
                    app_state.should_quit = true;
                    break;
                }
            }
        }

        // Check for serial data
        if let Ok(data) = serial_rx.try_recv() {
            match data {
                SerialData::Received(line) => {
                    app_state.add_output(line);
                }
            }
        }

        // Handle keyboard input
        if event::poll(Duration::from_millis(5))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        KeyCode::Char(c)
                            if k.modifiers.contains(KeyModifiers::CONTROL)
                                && (c == 'c' || c == 'd') =>
                        {
                            app_state.should_quit = true;
                            break;
                        }
                        KeyCode::Esc => {
                            app_state.should_quit = true;
                        }
                        KeyCode::Char('a') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            // Ctrl+A to re-enable auto-scroll
                            app_state.enable_auto_scroll();
                        }
                        KeyCode::Char(c) => {
                            app_state.input_line.push(c);
                        }
                        KeyCode::Enter => {
                            // Send the complete line to serial port
                            if !app_state.input_line.is_empty() {
                                write_bytes_async(&serial_config.port, app_state.input_line.as_bytes()).await?;
                                if let Some(w) = &ui_config.tx_log
                                    && let Ok(mut lw) = w.lock()
                                {
                                    if ui_config.log_ts {
                                        let _ = write!(lw, "[{}] ", now_rfc3339());
                                    }
                                    let _ = lw.write_all(app_state.input_line.as_bytes());
                                    let _ = lw.flush();
                                }
                            }

                            // Send line ending
                            let end = ui_config.line_ending.bytes();
                            if !end.is_empty() {
                                write_bytes_async(&serial_config.port, end).await?;
                                if let Some(w) = &ui_config.tx_log
                                    && let Ok(mut lw) = w.lock()
                                {
                                    if ui_config.log_ts && app_state.input_line.is_empty() {
                                        let _ = write!(lw, "[{}] ", now_rfc3339());
                                    }
                                    let _ = lw.write_all(end);
                                    let _ = lw.flush();
                                }
                            }

                            // Clear input for next line
                            app_state.input_line.clear();
                        }
                        KeyCode::Backspace => {
                            app_state.input_line.pop();
                        }
                        KeyCode::Up => {
                            app_state.scroll_up();
                        }
                        KeyCode::Down => {
                            app_state.scroll_down();
                        }
                        KeyCode::PageUp => {
                            // Disable auto-scroll and scroll up by 10 lines
                            app_state.auto_scroll = false;
                            let current = app_state
                                .list_state
                                .selected()
                                .unwrap_or(app_state.output_lines.len().saturating_sub(1));
                            let new_selected = current.saturating_sub(10);
                            if !app_state.output_lines.is_empty() {
                                app_state.list_state.select(Some(new_selected));
                            }
                        }
                        KeyCode::PageDown => {
                            // Disable auto-scroll and scroll down by 10 lines
                            app_state.auto_scroll = false;
                            let current = app_state.list_state.selected().unwrap_or(0);
                            let new_selected =
                                (current + 10).min(app_state.output_lines.len().saturating_sub(1));
                            if !app_state.output_lines.is_empty() {
                                app_state.list_state.select(Some(new_selected));
                            }
                        }
                        KeyCode::Home => {
                            app_state.scroll_to_home();
                        }
                        KeyCode::End => {
                            app_state.scroll_to_bottom();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Render the UI
        terminal.draw(|f| draw_ui(f, &mut app_state))?;
    }

    ui_config.running.store(false, Ordering::SeqCst);
    Ok(())
}

fn draw_ui(f: &mut Frame, app_state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Output area (takes most space)
            Constraint::Length(3), // Input area (fixed height)
        ])
        .split(f.area());

    // Serial monitor output
    let output_items: Vec<ListItem> = app_state
        .output_lines
        .iter()
        .map(|line| ListItem::new(line.as_str()))
        .collect();

    let title = if app_state.auto_scroll {
        "Serial Monitor (Auto-scroll ON - ↑↓/PgUp/PgDn to scroll, Ctrl+A to re-enable auto-scroll)"
    } else {
        "Serial Monitor (Auto-scroll OFF - ↑↓/PgUp/PgDn to scroll, Ctrl+A to re-enable auto-scroll)"
    };

    let output_list = List::new(output_items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::White));

    // Handle auto-scrolling vs manual scrolling
    if app_state.auto_scroll {
        // Use the persistent auto-scroll state that stays positioned at bottom
        f.render_stateful_widget(output_list, chunks[0], &mut app_state.auto_scroll_state);
    } else {
        // Manual scrolling mode - use the user's scroll position
        f.render_stateful_widget(output_list, chunks[0], &mut app_state.list_state);
    }

    // Input line
    let input_paragraph = Paragraph::new(app_state.input_line.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input (Press Enter to send, Ctrl+C or Esc to exit)"),
        )
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(input_paragraph, chunks[1]);

    // Set cursor position in input field
    f.set_cursor_position((
        chunks[1].x + app_state.input_line.len() as u16 + 1,
        chunks[1].y + 1,
    ));
}

async fn write_bytes_async(port: &Arc<Mutex<Box<dyn SerialPort + Send>>>, bytes: &[u8]) -> Result<()> {
    let mut guard = port.lock().await;
    guard.write_all(bytes)?;
    guard.flush()?;
    Ok(())
}
