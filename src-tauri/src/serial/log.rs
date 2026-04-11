// serial/log.rs – Background serial log reader.
//
// Opens a serial port, reads bytes in a loop, reassembles lines, and emits
// "log:line" Tauri events to the frontend.  A shared AtomicBool lets the main
// thread request graceful shutdown.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::AppState;

// ─── Event payloads ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub text: String,
    /// Milliseconds since the log session started.
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogStatus {
    pub connected: bool,
    pub message: String,
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Open `port` at `baud_rate` and start streaming "log:line" events.
/// Any previously running log session is stopped first.
pub fn start_log(
    app: AppHandle,
    state: &AppState,
    port: &str,
    baud_rate: u32,
) -> Result<(), String> {
    // Stop any existing session
    state.log_stop.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(150));
    state.log_stop.store(false, Ordering::SeqCst);

    // Try to open the port *before* spawning the thread so that errors surface
    // immediately to the caller.
    let mut serial = serialport::new(port, baud_rate)
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|e| format!("Cannot open {port}: {e}"))?;

    // Release DTR/RTS so we don't hold the device in reset while viewing logs.
    let _ = serial.write_data_terminal_ready(false);
    let _ = serial.write_request_to_send(false);

    let stop = Arc::clone(&state.log_stop);
    app.emit("log:status", LogStatus {
        connected: true,
        message: format!("Connected to {port} @ {baud_rate}"),
    }).ok();

    std::thread::spawn(move || run_log_thread(app, serial, stop));
    Ok(())
}

/// Signal the log thread to stop.
pub fn stop_log(state: &AppState) {
    state.log_stop.store(true, Ordering::SeqCst);
}

// ─── Background thread ────────────────────────────────────────────────────────

fn run_log_thread(
    app: AppHandle,
    mut port: Box<dyn serialport::SerialPort>,
    stop: Arc<AtomicBool>,
) {
    let start = std::time::Instant::now();
    let mut buf = vec![0u8; 4096];
    let mut line_buf: Vec<u8> = Vec::with_capacity(256);

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        match port.read(&mut buf) {
            Ok(0) => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' {
                        let text = String::from_utf8_lossy(&line_buf)
                            .trim_end_matches('\r')
                            .to_string();
                        line_buf.clear();
                        app.emit("log:line", LogLine {
                            text,
                            timestamp_ms: start.elapsed().as_millis() as u64,
                        }).ok();
                    } else {
                        line_buf.push(b);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                // Expected — the port timeout is 100 ms so we can check the stop flag.
            }
            Err(e) => {
                app.emit("log:status", LogStatus {
                    connected: false,
                    message: format!("Serial error: {e}"),
                }).ok();
                break;
            }
        }
    }

    // Flush any remaining partial line
    if !line_buf.is_empty() {
        app.emit("log:line", LogLine {
            text: String::from_utf8_lossy(&line_buf).trim_end_matches('\r').to_string(),
            timestamp_ms: start.elapsed().as_millis() as u64,
        }).ok();
    }

    app.emit("log:status", LogStatus {
        connected: false,
        message: "Disconnected".into(),
    }).ok();
}
