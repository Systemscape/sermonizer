use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use crossterm::{event, terminal};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serialport::{SerialPort, SerialPortType};
use std::fs::OpenOptions;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
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

#[derive(Copy, Clone, Debug, ValueEnum)]
enum NewlineMode {
    /// Send complete lines on Enter (Arduino Serial Monitor style)
    Onenter,
    /// Legacy mode: Send on Enter AND echo locally (so cursor moves)
    Ontype,
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

    /// How Enter behaves (onenter|ontype). Default: onenter
    #[arg(long, value_enum)]
    newline_mode: Option<NewlineMode>,

    /// Echo typed characters locally (TX)
    #[arg(long, default_value_t = true)]
    echo: bool,

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
    use std::time::SystemTime;
    use std::fmt;
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
        let era = (a >= 0).then_some(a).unwrap_or(a - 146096) / 146097;
        let doe = a - era * 146097;
        let yoe = (doe - doe/1460 + doe/36524 - doe/146096) / 365;
        let y = (yoe as i32) + era as i32 * 400;
        let doy = doe - (365*yoe + yoe/4 - yoe/100);
        let mp = (5*doy + 2)/153;
        let d = doy - (153*mp + 2)/5 + 1;
        let m = mp + if mp < 10 {3} else {-9};
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

fn main() -> Result<()> {
    let args = Args::parse();

    // Enumerate ports up front
    let all_ports = serialport::available_ports()
        .context("Failed to list serial ports")?;
    
    // Filter for realistic ports (USB ports with VID/PID)
    let ports: Vec<_> = all_ports.into_iter()
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

    // Line ending + mode
    let line_ending = args.line_ending.unwrap_or(LineEnding::Nl);
    if args.line_ending.is_none() {
        println!("Line ending: {} (default)", line_ending.describe());
    } else {
        println!("Line ending: {}", line_ending.describe());
    }
    let newline_mode = args.newline_mode.unwrap_or(NewlineMode::Onenter);
    println!(
        "Newline mode: {}",
        match newline_mode {
            NewlineMode::Onenter => "onenter (build lines locally, send complete lines on Enter)",
            NewlineMode::Ontype => "ontype (legacy mode - send on Enter and echo locally)",
        }
    );

    if args.echo { println!("Local echo: ON"); }
    if args.hex { println!("RX view: HEX"); }
    if args.log_ts { println!("Timestamps in logs: ON"); }

    // Open port
    let port = serialport::new(&port_name, baud)
        .timeout(Duration::from_millis(10))
        .open()
        .with_context(|| format!("Failed to open serial port '{port_name}'"))?;

    println!("Connected. Type to send; press Ctrl-C to exit.\n");

    // Shared port between reader/writer
    let port: Arc<Mutex<Box<dyn SerialPort + Send>>> = Arc::new(Mutex::new(port));

    // Optional log files
    let rx_log_writer: Option<Arc<Mutex<BufWriter<std::fs::File>>>> = match &args.log {
        Some(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("Failed to open log file: {}", path.display()))?;
            println!("Logging RX to: {}", path.display());
            Some(Arc::new(Mutex::new(BufWriter::new(file))))
        }
        None => None,
    };
    let tx_log_writer: Option<Arc<Mutex<BufWriter<std::fs::File>>>> = match &args.tx_log {
        Some(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("Failed to open tx-log file: {}", path.display()))?;
            println!("Logging TX to: {}", path.display());
            Some(Arc::new(Mutex::new(BufWriter::new(file))))
        }
        None => None,
    };

    // Handle Ctrl-C
    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        ctrlc::set_handler(move || {
            running.store(false, Ordering::SeqCst);
        }).expect("Failed to set Ctrl-C handler");
    }

    // Spawn reader thread (RX)
    let port_reader = port.clone();
    let running_reader = running.clone();
    let rx_log_writer_reader = rx_log_writer.clone();
    let hex_mode = args.hex;
    let log_ts = args.log_ts;
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        let mut stdout = std::io::stdout();

        while running_reader.load(Ordering::SeqCst) {
            let n = {
                let mut guard = match port_reader.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match guard.read(&mut buf) {
                    Ok(n) => n,
                    Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => 0,
                    Err(_) => break,
                }
            };

            if n > 0 {
                let bytes = &buf[..n];

                // Terminal output
                if hex_mode {
                    // Timestamp prefix once per chunk
                    if log_ts {
                        let _ = write!(stdout, "[{}] ", now_rfc3339());
                    }
                    for (i, b) in bytes.iter().enumerate() {
                        let _ = write!(stdout, "{:02X}{}", b, if i + 1 == bytes.len() { "" } else { " " });
                    }
                    let _ = writeln!(stdout);
                    let _ = stdout.flush();
                } else {
                    let _ = stdout.write_all(bytes);
                    let _ = stdout.flush();
                }

                // RX log file
                if let Some(w) = &rx_log_writer_reader {
                    if let Ok(mut lw) = w.lock() {
                        if log_ts {
                            let _ = write!(lw, "[{}] ", now_rfc3339());
                        }
                        if hex_mode {
                            for (i, b) in bytes.iter().enumerate() {
                                let _ = write!(lw, "{:02X}{}", b, if i + 1 == bytes.len() { "" } else { " " });
                            }
                            let _ = writeln!(lw);
                        } else {
                            let _ = lw.write_all(bytes);
                        }
                        let _ = lw.flush();
                    }
                }
            }
        }
    });

    // Enter raw mode to capture keys immediately
    terminal::enable_raw_mode().context("Failed to enable raw mode")?;
    let writer_res = run_key_loop(
        port.clone(),
        running.clone(),
        line_ending,
        newline_mode,
        args.echo,
        tx_log_writer.clone(),
        args.log_ts,
    );
    let _ = terminal::disable_raw_mode();

    // Ensure we stop and join reader
    running.store(false, Ordering::SeqCst);
    let _ = reader.join();

    if let Err(e) = writer_res {
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
                if let Some(m) = &info.manufacturer { print!(" {m}"); }
                if let Some(pn) = &info.product { print!(" {pn}"); }
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
            if was_raw { let _ = crossterm::terminal::disable_raw_mode(); }

            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            if was_raw { let _ = crossterm::terminal::enable_raw_mode(); }

            let sel = line.trim().parse::<usize>().unwrap_or(1);
            let idx = sel.clamp(1, ports.len()) - 1;
            let name = ports[idx].port_name.clone();
            println!("Using port: {name}");
            Ok(name)
        }
    }
}

fn run_key_loop(
    port: Arc<Mutex<Box<dyn SerialPort + Send>>>,
    running: Arc<AtomicBool>,
    line_ending: LineEnding,
    _newline_mode: NewlineMode,
    echo: bool,
    tx_log: Option<Arc<Mutex<BufWriter<std::fs::File>>>>,
    log_ts: bool,
) -> Result<()> {
    let mut stdout = std::io::stdout();
    let mut line_buffer = String::new();

    // Key event loop — build lines locally, send on Enter like Arduino Serial Monitor
    while running.load(Ordering::SeqCst) {
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        KeyCode::Char(c) if k.modifiers.contains(KeyModifiers::CONTROL) && (c == 'c' || c == 'd' || c == 'z') => {
                            // Handle Ctrl+C, Ctrl+D, Ctrl+Z
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                        KeyCode::Char(c) => {
                            line_buffer.push(c);
                            if echo {
                                let _ = write!(stdout, "{}", c);
                                let _ = stdout.flush();
                            }
                        }
                        KeyCode::Enter => {
                            // Send the complete line to serial port
                            if !line_buffer.is_empty() {
                                write_bytes(&port, line_buffer.as_bytes())?;
                                if let Some(w) = &tx_log {
                                    if let Ok(mut lw) = w.lock() {
                                        if log_ts { let _ = write!(lw, "[{}] ", now_rfc3339()); }
                                        let _ = lw.write_all(line_buffer.as_bytes());
                                        let _ = lw.flush();
                                    }
                                }
                            }
                            
                            // Send line ending
                            let end = line_ending.bytes();
                            if !end.is_empty() {
                                write_bytes(&port, end)?;
                                if let Some(w) = &tx_log {
                                    if let Ok(mut lw) = w.lock() {
                                        if log_ts && line_buffer.is_empty() { let _ = write!(lw, "[{}] ", now_rfc3339()); }
                                        let _ = lw.write_all(end);
                                        let _ = lw.flush();
                                    }
                                }
                            }
                            
                            // Local echo newline
                            if echo {
                                let _ = writeln!(stdout);
                                let _ = stdout.flush();
                            }
                            
                            // Clear buffer for next line
                            line_buffer.clear();
                        }
                        KeyCode::Backspace => {
                            if !line_buffer.is_empty() {
                                line_buffer.pop();
                                if echo {
                                    let _ = write!(stdout, "\x08 \x08");
                                    let _ = stdout.flush();
                                }
                            }
                        }
                        KeyCode::Esc => {
                            // Optional: allow ESC to exit quickly
                            running.store(false, Ordering::SeqCst);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn write_bytes(port: &Arc<Mutex<Box<dyn SerialPort + Send>>>, bytes: &[u8]) -> Result<()> {
    let mut guard = match port.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.write_all(bytes)?;
    guard.flush()?;
    Ok(())
}
