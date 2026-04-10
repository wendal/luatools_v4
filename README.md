# LuatOS Flashing Tool (LuaTools v4)

A modern Tauri 2 + Vue 3 + Rust desktop application for flashing LuatOS firmware onto
EC618 / EC718 (Air780E / Air780EP) and Beken-based modules.

## Features

- Clean dark-mode UI built with Vue 3 + Tailwind CSS
- Automatic serial port enumeration on startup and Refresh
- EC618 / EC718 flasher backend (Rust, work-in-progress)
- Beken (BK) flasher backend (Rust, work-in-progress)

## Getting Started

### Prerequisites

| Tool | Version |
|------|---------|
| Rust (via rustup) | stable ≥ 1.77 |
| Node.js | ≥ 18 LTS |
| Tauri CLI | `npm run tauri --version` |

```bash
# Install Node dependencies (includes @tauri-apps/api)
npm install

# Start the development build (Vite dev-server + Tauri window)
npm run tauri:dev

# Production build
npm run tauri:build
```

## Debugging Serial-Port Enumeration

If the **Device** dropdown stays empty after the app starts or after clicking **Refresh**,
work through the checklist below.

### 1 — Enable Rust-level debug logs

On **Windows** (PowerShell):

```powershell
$env:RUST_LOG = "debug"
npm run tauri:dev
```

On **macOS / Linux**:

```bash
RUST_LOG=debug npm run tauri:dev
```

Look for lines like:

```
[serial] Raw port count from OS: 3
[serial]   port=COM3 vid=Some("1a86") pid=Some("7523") product=Some("USB Serial")
[serial] Returning 3 port(s) to frontend
```

If the count is **0**, the issue is at the OS / driver level (see step 3).
If count is > 0 but the UI still shows nothing, the issue is in the frontend IPC
call (check the browser DevTools console for errors from `invoke('get_serial_ports')`).

### 2 — Check browser DevTools

Press **F12** in the Tauri window (or set `"devtools": true` in `tauri.conf.json`)
to open DevTools. Look for:

```
[FlashView] get_serial_ports returned: [...]
```

Any error message printed there points directly to the IPC or permission layer.

### 3 — Verify the OS sees the port

Run the `serialport` crate's own example directly to confirm whether the problem
is in hardware / drivers or in the application:

```bash
# In the src-tauri directory
cargo run --example list_ports
```

> **Note**: the example must be added to `Cargo.toml` first — see the
> [serialport documentation](https://docs.rs/serialport/latest/serialport/).
> Alternatively, use the `serial-monitor` tool:

```bash
cargo install serial-monitor
```

On Windows you can also check **Device Manager → Ports (COM & LPT)**.
If the port is listed there but not enumerated by the app, it is likely a
**driver or permissions issue** (try running the app as administrator once to
confirm).

### 4 — Common root causes

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Count = 0 in Rust log | No device connected / wrong driver | Install CH340 / CP210x driver; check Device Manager |
| Count > 0 but UI empty | `@tauri-apps/api` version mismatch | Verify `npm list @tauri-apps/api` matches `@tauri-apps/cli` |
| `invoke` throws "command not found" | Command not registered | Confirm `get_serial_ports` appears in `tauri::generate_handler![]` |
| Port visible in Device Manager, absent in enumeration | Windows filter driver conflict | Reboot; remove and re-install USB driver |