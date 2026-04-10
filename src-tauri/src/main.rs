// Prevents an additional console window from appearing on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod flash;
mod luadb;
mod serial;

use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use serial::PortInfo;

// ─── Application State ───────────────────────────────────────────────────────

pub struct AppState {
    /// Legacy field kept for compatibility; no longer used for BK flash.
    pub flash_child: Arc<Mutex<Option<std::process::Child>>>,
    /// Set to `true` to cancel an in-progress flash.
    pub flash_cancel: Arc<AtomicBool>,
    /// Set to `true` to ask the serial log thread to exit.
    pub log_stop: Arc<AtomicBool>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            flash_child: Arc::new(Mutex::new(None)),
            flash_cancel: Arc::new(AtomicBool::new(false)),
            log_stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

// ─── Serial port enumeration ─────────────────────────────────────────────────

#[tauri::command]
fn get_serial_ports() -> Vec<PortInfo> {
    serial::list_ports()
}

// ─── File / folder dialogs ───────────────────────────────────────────────────

#[tauri::command]
fn open_file_dialog(
    title: String,
    filter_name: String,
    extensions: Vec<String>,
) -> Option<String> {
    let exts: Vec<&str> = extensions.iter().map(|s| s.as_str()).collect();
    rfd::FileDialog::new()
        .set_title(&title)
        .add_filter(&filter_name, &exts)
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_folder_dialog(title: String) -> Option<String> {
    rfd::FileDialog::new()
        .set_title(&title)
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
}

// ─── .soc info ───────────────────────────────────────────────────────────────

#[tauri::command]
fn get_soc_info(soc_path: String) -> Result<flash::bk::SocInfo, String> {
    flash::bk::read_soc_info(&soc_path).map_err(|e| e.to_string())
}

// ─── Flashing ────────────────────────────────────────────────────────────────

#[tauri::command]
async fn flash_device(
    app: AppHandle,
    state: State<'_, AppState>,
    soc_path: String,
    script_folder: Option<String>,
    port: String,
    baud_rate: Option<u32>,
) -> Result<(), String> {
    let flash_child = Arc::clone(&state.flash_child);
    let flash_cancel = Arc::clone(&state.flash_cancel);
    tokio::task::spawn_blocking(move || {
        flash::bk::flash_bk7258(
            app,
            flash_child,
            &soc_path,
            script_folder.as_deref(),
            &port,
            baud_rate,
            flash_cancel,
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map(|_| ())  // discard returned boot log lines
    .map_err(|e: anyhow::Error| e.to_string())
}

#[tauri::command]
fn cancel_flash(state: State<'_, AppState>) {
    state.flash_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    // Also kill any legacy subprocess
    let mut guard = state.flash_child.lock().unwrap();
    if let Some(ref mut child) = *guard {
        let _ = child.kill();
        let _ = child.wait();
    }
    *guard = None;
}

// ─── Serial log ──────────────────────────────────────────────────────────────

#[tauri::command]
fn start_serial_log(
    app: AppHandle,
    state: State<'_, AppState>,
    port: String,
    baud_rate: u32,
) -> Result<(), String> {
    serial::log::start_log(app, &state, &port, baud_rate)
}

#[tauri::command]
fn stop_serial_log(state: State<'_, AppState>) {
    serial::log::stop_log(&state);
}

// ─── Auto flash+test ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct TestResult {
    pub passed: bool,
    pub lines: Vec<String>,
    pub message: String,
}

/// Flash the device and capture its boot log atomically.
/// The subprocess flash (air602_flash.exe) sends Boot_Reboot at the end of
/// flashing, which causes the device to reboot.  We open the log port immediately
/// after the subprocess exits and read output for `timeout_secs` seconds.
/// Returns PASS if boot keywords are found in the captured log.
#[tauri::command]
async fn run_flash_test(
    app: AppHandle,
    state: State<'_, AppState>,
    soc_path: String,
    port: String,
    timeout_secs: Option<u64>,
) -> Result<TestResult, String> {
    let _ = timeout_secs; // boot log capture duration is controlled by LOG_CAPTURE_SECS in bk.rs
    let flash_child = Arc::clone(&state.flash_child);
    let flash_cancel = Arc::clone(&state.flash_cancel);
    let soc2 = soc_path.clone();
    let port2 = port.clone();
    let app2 = app.clone();

    // flash_bk7258 handles both flash AND boot log capture in one blocking operation.
    // The boot log capture starts immediately after the subprocess exits — no delay.
    let lines = tokio::task::spawn_blocking(move || {
        flash::bk::flash_bk7258(app2, flash_child, &soc2, None, &port2, None, flash_cancel)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e: anyhow::Error| e.to_string())?;

    let boot_kw = ["luat:", "ap0:", "ap1:", "MAC ", "EasyFlash", "LuatOS", "RTOS"];
    let passed = lines.iter().any(|l| boot_kw.iter().any(|kw| l.contains(kw)));
    let message = if passed {
        format!("PASS — {} log lines, boot strings detected", lines.len())
    } else {
        format!("FAIL — {} log lines, no boot strings found", lines.len())
    };

    Ok(TestResult { passed, lines, message })
}

// ─── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_serial_ports,
            open_file_dialog,
            open_folder_dialog,
            get_soc_info,
            flash_device,
            cancel_flash,
            start_serial_log,
            stop_serial_log,
            run_flash_test,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

