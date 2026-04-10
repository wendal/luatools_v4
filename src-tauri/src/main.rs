// Prevents an additional console window from appearing on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod flash;
mod luadb;
mod serial;

use serial::PortInfo;

/// Tauri command: return a list of all serial ports visible on the host.
///
/// Invoked from the Vue 3 front-end via `invoke("get_serial_ports")`.
/// Returns a JSON array of [`PortInfo`] objects; never throws on the JS side
/// because errors are handled in Rust and surfaced as an empty array.
#[tauri::command]
fn get_serial_ports() -> Vec<PortInfo> {
    serial::list_ports()
}

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_serial_ports])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
