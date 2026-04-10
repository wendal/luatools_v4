//! bk-cli — Standalone CLI for BK7258/Air8101 firmware operations.
//!
//! Usage:
//!   bk-cli ports
//!   bk-cli flash --soc <path> --port <port>
//!   bk-cli log   --port <port> [--baud <rate>]
//!   bk-cli test  --soc <path> --port <port> [--timeout <secs>]

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::time::{Duration, Instant};

// ─── Timestamp helper ────────────────────────────────────────────────────────

fn ts() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

macro_rules! info {
    ($($arg:tt)*) => { println!("[{}] {}", ts(), format!($($arg)*)); };
}
macro_rules! warn {
    ($($arg:tt)*) => { println!("[{} WARN] {}", ts(), format!($($arg)*)); };
}
macro_rules! err {
    ($($arg:tt)*) => { eprintln!("[{} ERROR] {}", ts(), format!($($arg)*)); };
}

// ─── Minimal info.json schema ────────────────────────────────────────────────

#[derive(serde::Deserialize, Debug)]
struct SocInfo {
    rom: SocRom,
    download: SocDownload,
    user: Option<SocUser>,
}

#[derive(serde::Deserialize, Debug)]
struct SocRom {
    file: String,
}

#[derive(serde::Deserialize, Debug)]
struct SocDownload {
    force_br: Option<String>,
    #[allow(dead_code)]
    bl_addr: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct SocUser {
    log_br: Option<String>,
}

// ─── Argument parsing ────────────────────────────────────────────────────────

fn print_usage() {
    println!(
        r#"bk-cli — BK7258/Air8101 firmware tool

Usage:
  bk-cli ports                                  List serial ports
  bk-cli flash --soc <path> --port <port>       Flash firmware
  bk-cli log   --port <port> [--baud <rate>]    View serial log
  bk-cli test  --soc <path> --port <port>       Flash + verify boot
               [--timeout <secs>]

Options:
  --soc <path>      Path to .soc firmware file
  --port <port>     Serial port (e.g. COM6)
  --baud <rate>     Baud rate for log (default: 2000000)
  --timeout <secs>  Boot log capture window (default: 20)
"#
    );
}

fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}

fn require_arg(args: &[String], flag: &str) -> String {
    get_arg(args, flag).unwrap_or_else(|| {
        err!("{} is required", flag);
        std::process::exit(1);
    })
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let result = match args[1].as_str() {
        "ports" => cmd_ports(),
        "flash" => {
            let soc = require_arg(&args, "--soc");
            let port = require_arg(&args, "--port");
            cmd_flash(&soc, &port)
        }
        "log" => {
            let port = require_arg(&args, "--port");
            let baud: u32 = get_arg(&args, "--baud")
                .and_then(|s| s.parse().ok())
                .unwrap_or(2_000_000);
            cmd_log(&port, baud)
        }
        "test" => {
            let soc = require_arg(&args, "--soc");
            let port = require_arg(&args, "--port");
            let timeout: u64 = get_arg(&args, "--timeout")
                .and_then(|s| s.parse().ok())
                .unwrap_or(20);
            cmd_test(&soc, &port, timeout)
        }
        "--help" | "-h" | "help" => {
            print_usage();
            Ok(())
        }
        other => {
            err!("Unknown command: {other}");
            print_usage();
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        err!("{e}");
        std::process::exit(1);
    }
}

// ─── Command: ports ──────────────────────────────────────────────────────────

fn cmd_ports() -> Result<(), String> {
    info!("Enumerating serial ports...");
    let ports = serialport::available_ports().map_err(|e| format!("Failed: {e}"))?;
    if ports.is_empty() {
        info!("No serial ports found.");
        return Ok(());
    }
    for p in &ports {
        let extra = match &p.port_type {
            serialport::SerialPortType::UsbPort(info) => {
                format!(
                    "  USB VID:{:04x} PID:{:04x} {}",
                    info.vid,
                    info.pid,
                    info.product.as_deref().unwrap_or("")
                )
            }
            _ => String::new(),
        };
        println!("  {}{}", p.port_name, extra);
    }
    info!("{} port(s) found.", ports.len());
    Ok(())
}

// ─── SOC extraction ──────────────────────────────────────────────────────────

fn extract_soc(soc_path: &str) -> Result<(tempfile::TempDir, SocInfo), String> {
    info!("Extracting SOC: {soc_path}");
    let file =
        std::fs::File::open(soc_path).map_err(|e| format!("Cannot open SOC file: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Not a valid SOC (zip): {e}"))?;
    let tempdir =
        tempfile::tempdir().map_err(|e| format!("Cannot create temp dir: {e}"))?;
    archive
        .extract(tempdir.path())
        .map_err(|e| format!("Extraction failed: {e}"))?;

    let info: SocInfo = {
        let f = std::fs::File::open(tempdir.path().join("info.json"))
            .map_err(|e| format!("info.json not found: {e}"))?;
        serde_json::from_reader(f).map_err(|e| format!("Parse info.json failed: {e}"))?
    };

    info!("  ROM file  : {}", info.rom.file);
    info!(
        "  Flash baud: {}",
        info.download.force_br.as_deref().unwrap_or("default")
    );
    info!(
        "  Log baud  : {}",
        info.user
            .as_ref()
            .and_then(|u| u.log_br.as_deref())
            .unwrap_or("2000000")
    );

    Ok((tempdir, info))
}

// ─── Subprocess flash ────────────────────────────────────────────────────────

fn run_flash_exe(exe_path: &Path, rom_path: &Path, port: &str) -> Result<(), String> {
    let port_num: String = port.chars().filter(|c| c.is_ascii_digit()).collect();
    if port_num.is_empty() {
        return Err(format!("Invalid port name: {port}"));
    }

    info!("Launching air602_flash.exe on port {port} ...");
    println!("─── air602_flash.exe output ───────────────────────────");

    let mut child = std::process::Command::new(exe_path)
        .args([
            "download",
            "-p",
            &port_num,
            "-b",
            "2000000",
            "-s",
            "0",
            "-i",
        ])
        .arg(rom_path)
        .current_dir(exe_path.parent().unwrap_or(Path::new(".")))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch air602_flash.exe: {e}"))?;

    // Drain stdout in a background thread so the pipe doesn't block
    let stdout = child.stdout.take().expect("stdout was piped");
    let drain = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() {
            let t = line.trim().to_string();
            if !t.is_empty() {
                println!("[{}] [EXE] {}", ts(), t);
            }
        }
    });

    // Block until subprocess exits — returns the instant it terminates.
    // This is the critical synchronization point for boot log capture.
    let status = child.wait().map_err(|e| format!("wait() failed: {e}"))?;

    let _ = drain.join();
    println!("───────────────────────────────────────────────────────");
    info!(
        "air602_flash.exe exited (code {:?})",
        status.code()
    );

    Ok(())
}

// ─── Boot log capture ────────────────────────────────────────────────────────

fn capture_boot_log(
    port: &str,
    baud: u32,
    timeout_secs: u64,
) -> Result<Vec<String>, String> {
    info!("Opening {port} @ {baud} for boot log ({timeout_secs}s window)...");

    // Retry open — Windows COM driver may not have released yet
    let mut serial = {
        let mut last_err = String::new();
        let mut ok = None;
        for i in 0..20 {
            match serialport::new(port, baud)
                .timeout(Duration::from_millis(200))
                .open()
            {
                Ok(p) => {
                    ok = Some(p);
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    if i == 0 {
                        warn!("Port not ready, retrying... ({last_err})");
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        ok.ok_or_else(|| format!("Cannot open {port}: {last_err}"))?
    };

    info!("Port opened! Capturing boot log...");
    println!("─── Boot Log ─────────────────────────────────────────");

    let mut log_bytes: Vec<u8> = Vec::new();
    let mut line_buf: Vec<u8> = Vec::new();
    let mut read_buf = [0u8; 512];
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let start = Instant::now();
    let mut first_data = true;

    while Instant::now() < deadline {
        match serial.read(&mut read_buf) {
            Ok(n) if n > 0 => {
                if first_data {
                    info!(
                        "First data arrived at +{:.3}s after port open",
                        start.elapsed().as_secs_f64()
                    );
                    first_data = false;
                }
                log_bytes.extend_from_slice(&read_buf[..n]);
                for &b in &read_buf[..n] {
                    if b == b'\n' {
                        let s = String::from_utf8_lossy(&line_buf)
                            .trim_end_matches('\r')
                            .to_string();
                        line_buf.clear();
                        if !s.is_empty() {
                            println!("{}", s);
                        }
                    } else {
                        line_buf.push(b);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                warn!("Serial read error: {e}");
                break;
            }
            _ => {}
        }
    }

    // Flush any partial line
    if !line_buf.is_empty() {
        let s = String::from_utf8_lossy(&line_buf)
            .trim_end_matches('\r')
            .to_string();
        if !s.is_empty() {
            println!("{}", s);
        }
    }

    println!("───────────────────────────────────────────────────────");

    let log_text = String::from_utf8_lossy(&log_bytes);
    let lines: Vec<String> = log_text
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect();

    info!("Captured {} bytes, {} lines", log_bytes.len(), lines.len());
    Ok(lines)
}

// ─── Command: flash ──────────────────────────────────────────────────────────

fn cmd_flash(soc_path: &str, port: &str) -> Result<(), String> {
    info!("═══ BK7258 FLASH ═══");
    let total = Instant::now();

    let (tempdir, info) = extract_soc(soc_path)?;

    let rom_path = tempdir.path().join(&info.rom.file);
    if !rom_path.exists() {
        return Err(format!("ROM '{}' not found in SOC", info.rom.file));
    }

    let exe_path = tempdir.path().join("air602_flash.exe");
    if !exe_path.exists() {
        return Err("air602_flash.exe not found in SOC archive".into());
    }

    let flash_start = Instant::now();
    run_flash_exe(&exe_path, &rom_path, port)?;
    let flash_secs = flash_start.elapsed().as_secs_f64();
    info!("Flash completed in {flash_secs:.1}s");

    // Capture boot log
    let log_br: u32 = info
        .user
        .as_ref()
        .and_then(|u| u.log_br.as_deref())
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);

    let lines = capture_boot_log(port, log_br, 20)?;

    let boot_kw = ["luat:", "ap0:", "ap1:", "LuatOS", "EasyFlash", "MAC "];
    let found: Vec<&str> = boot_kw
        .iter()
        .filter(|kw| lines.iter().any(|l| l.contains(**kw)))
        .copied()
        .collect();

    if !found.is_empty() {
        info!("Boot keywords found: {:?}", found);
        info!("✓ Device booted successfully! ({:.1}s total)", total.elapsed().as_secs_f64());
    } else {
        warn!("No boot keywords found in log output.");
    }

    Ok(())
}

// ─── Command: log ────────────────────────────────────────────────────────────

fn cmd_log(port: &str, baud: u32) -> Result<(), String> {
    info!("Opening {port} @ {baud} (Ctrl+C to exit)...");

    let mut serial = serialport::new(port, baud)
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|e| format!("Cannot open {port}: {e}"))?;

    // Release DTR/RTS so we don't hold the device in reset
    let _ = serial.write_data_terminal_ready(false);
    let _ = serial.write_request_to_send(false);

    info!("Connected. Streaming serial output...");
    println!("─── Serial Log (Ctrl+C to exit) ──────────────────────");

    let mut line_buf: Vec<u8> = Vec::new();
    let mut read_buf = [0u8; 4096];

    loop {
        match serial.read(&mut read_buf) {
            Ok(n) if n > 0 => {
                for &b in &read_buf[..n] {
                    if b == b'\n' {
                        let s = String::from_utf8_lossy(&line_buf)
                            .trim_end_matches('\r')
                            .to_string();
                        line_buf.clear();
                        if !s.is_empty() {
                            println!("[{}] {}", ts(), s);
                        }
                    } else {
                        line_buf.push(b);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => {
                return Err(format!("Serial error: {e}"));
            }
            _ => {}
        }
    }
}

// ─── Command: test ───────────────────────────────────────────────────────────

fn cmd_test(soc_path: &str, port: &str, timeout_secs: u64) -> Result<(), String> {
    info!("══════════════════════════════════════════════════════");
    info!("  BK7258 CLOSED-LOOP TEST");
    info!("══════════════════════════════════════════════════════");
    let total = Instant::now();

    // ── Phase 1: Extract SOC ──
    info!("── Phase 1: Extracting SOC ──");
    let (tempdir, info) = extract_soc(soc_path)?;

    let rom_path = tempdir.path().join(&info.rom.file);
    if !rom_path.exists() {
        return Err(format!("ROM '{}' not found in SOC", info.rom.file));
    }

    let exe_path = tempdir.path().join("air602_flash.exe");
    if !exe_path.exists() {
        return Err("air602_flash.exe not found in SOC archive".into());
    }

    // ── Phase 2: Flash ──
    info!("── Phase 2: Flashing firmware ──");
    let flash_start = Instant::now();
    run_flash_exe(&exe_path, &rom_path, port)?;
    let flash_secs = flash_start.elapsed().as_secs_f64();
    info!("Flash phase done in {flash_secs:.1}s");

    // ── Phase 3: Boot log capture ──
    info!("── Phase 3: Capturing boot log ({timeout_secs}s) ──");
    let log_br: u32 = info
        .user
        .as_ref()
        .and_then(|u| u.log_br.as_deref())
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);

    let lines = capture_boot_log(port, log_br, timeout_secs)?;

    // ── Phase 4: Verify ──
    info!("── Phase 4: Verification ──");
    let boot_kw = [
        "luat:", "ap0:", "ap1:", "LuatOS", "EasyFlash", "MAC ", "RTOS",
    ];
    let found: Vec<&str> = boot_kw
        .iter()
        .filter(|kw| lines.iter().any(|l| l.contains(**kw)))
        .copied()
        .collect();

    let total_secs = total.elapsed().as_secs_f64();

    println!();
    println!("══════════════════════════════════════════════════════");
    if !found.is_empty() {
        info!("Keywords matched: {:?}", found);
        println!("  RESULT : ✓ PASS");
        println!("  Lines  : {}", lines.len());
        println!("  Time   : {total_secs:.1}s (flash {flash_secs:.1}s)");
        println!("══════════════════════════════════════════════════════");
        Ok(())
    } else {
        println!("  RESULT : ✗ FAIL");
        println!("  Lines  : {}", lines.len());
        println!("  Time   : {total_secs:.1}s");
        println!("  Reason : No boot keywords found in log");
        println!("══════════════════════════════════════════════════════");
        Err("Test FAILED: no boot keywords detected".into())
    }
}
