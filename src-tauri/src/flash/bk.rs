// flash/bk.rs – BK7258 (Air8101) native serial flasher.
//
// Protocol reference: https://github.com/openshwprojects/BK7231GUIFlashTool
//
// Flash sequence:
//   1. Extract .soc (zip) to temp dir; parse info.json.
//   2. Open COM port at 115 200 baud.
//   3. Toggle DTR+RTS to reset device into ROM bootloader.
//   4. LinkCheck handshake until device responds.
//   5. Switch to 2 Mbps (or force_br from info.json).
//   6. Read Flash MID; unprotect (clear BP/CMP bits in SR).
//   7. Erase sectors (4 K or 64 K blocks), write 4 K sectors.
//   8. Optionally flash script partition.
//   9. Close port → device auto-reboots.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const SECTOR_SIZE: usize = 0x1000;       // 4 KiB
const SECTORS_PER_BLOCK: usize = 16;     // 64 KiB / 4 KiB

// ─── info.json schema ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocInfo {
    pub version: Option<u32>,
    pub chip: SocChip,
    pub rom: SocRom,
    pub script: SocScript,
    pub download: SocDownload,
    pub user: Option<SocUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocChip {
    #[serde(rename = "type")]
    pub chip_type: String,
    pub ram: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocRom {
    pub file: String,
    pub fs: Option<SocFs>,
    #[serde(rename = "version-bsp")]
    pub version_bsp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocFs {
    pub script: Option<SocScriptFs>,
    pub filesystem: Option<serde_json::Value>,
    pub kv: Option<serde_json::Value>,
    pub ap: Option<serde_json::Value>,
    pub fota: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocScriptFs {
    pub offset: Option<String>,
    pub size: Option<u64>,
    #[serde(rename = "type")]
    pub fs_type: Option<String>,
    pub bkcrc: Option<bool>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocScript {
    pub file: String,
    pub lua: Option<String>,
    pub bitw: Option<u32>,
    #[serde(rename = "use-luac")]
    pub use_luac: Option<bool>,
    #[serde(rename = "use-debug")]
    pub use_debug: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocDownload {
    pub bl_addr: Option<String>,
    pub script_addr: Option<String>,
    pub force_br: Option<String>,
    pub cp_addr: Option<String>,
    pub ap_addr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocUser {
    pub log_br: Option<String>,
    pub project: Option<String>,
    pub version: Option<String>,
}

// ─── Tauri event payload ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct FlashEvent {
    pub stage: String,
    /// 0–100 progress percent; -1 means "informational, do not update bar".
    pub percent: f32,
    pub message: String,
    pub done: bool,
    pub error: bool,
}

impl FlashEvent {
    fn info(stage: &str, pct: f32, msg: &str) -> Self {
        Self { stage: stage.into(), percent: pct, message: msg.into(), done: false, error: false }
    }
    fn log_line(msg: &str) -> Self {
        Self { stage: String::new(), percent: -1.0, message: msg.into(), done: false, error: false }
    }
    pub fn done_ok(msg: &str) -> Self {
        Self { stage: "Done".into(), percent: 100.0, message: msg.into(), done: true, error: false }
    }
    pub fn done_err(msg: &str) -> Self {
        Self { stage: "Error".into(), percent: 0.0, message: msg.into(), done: true, error: true }
    }
}

// ─── Low-level serial helpers ─────────────────────────────────────────────────

/// Read exactly `buf.len()` bytes within `timeout`, returning error on timeout.
fn read_exact_timeout(
    port: &mut dyn serialport::SerialPort,
    buf: &mut [u8],
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut n = 0;
    while n < buf.len() {
        if Instant::now() > deadline {
            bail!("Serial read timeout: got {}/{} bytes", n, buf.len());
        }
        match port.read(&mut buf[n..]) {
            Ok(k) if k > 0 => n += k,
            Ok(_) | Err(_) => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    Ok(())
}

// ─── BK7231 protocol commands ─────────────────────────────────────────────────
//
// Short command (≤255 body bytes):
//   TX: [0x01, 0xE0, 0xFC, LEN, CMD, DATA...]  where LEN = 1 + len(DATA)
//   RX: [0x04, 0x0E, LEN, 0x01, 0xE0, 0xFC, CMD, DATA...]
//
// Long command (body > 255 bytes or flash operations):
//   TX: [0x01, 0xE0, 0xFC, 0xFF, 0xF4, LEN_LO, LEN_HI, CMD, DATA...]
//       where LEN (16-bit LE) = 1 + len(DATA)
//   RX: [0x04, 0x0E, 0xFF, 0x01, 0xE0, 0xFC, 0xF4, LEN_LO, LEN_HI, CMD, ...]
//       response body starts at index 9, length = LEN bytes

/// Send LinkCheck and verify response. Returns true if device ACKed.
/// Uses a short 2ms window — the bootloader responds within ~8ms after the reset pulse.
/// IMPORTANT: port timeout must be 1ms (set in get_bus) for rapid polling.
fn link_check_once(port: &mut dyn serialport::SerialPort) -> bool {
    let _ = port.clear(serialport::ClearBuffer::Input);
    // Short CMD=0x00, LEN=1, no data
    let tx = [0x01u8, 0xE0, 0xFC, 0x01, 0x00];
    if port.write_all(&tx).is_err() || port.flush().is_err() { return false; }
    let mut buf = [0u8; 8];
    let deadline = Instant::now() + Duration::from_millis(2);
    let mut n = 0;
    while n < 8 && Instant::now() < deadline {
        match port.read(&mut buf[n..]) {
            Ok(k) if k > 0 => n += k,
            _ => {}
        }
    }
    n >= 8 && buf == [0x04, 0x0E, 0x05, 0x01, 0xE0, 0xFC, 0x01, 0x00]
}

/// Toggle DTR+RTS to force device into ROM bootloader, then wait for LinkCheck ACK.
/// Uses the 5-phase pulse pattern from bk72xx_uart_flasher.py (luatools_py3 reference):
///   init to LOW → (DTR=1,RTS=1,50ms) → (0,0,20ms) → (1,0,50ms) → (0,1,50ms) → (0,0,20ms) → link checks
/// Phase 4 (DTR=0, RTS=1) is the key: device released from reset while in bootloader mode.
///
/// KEY FIX: port read timeout MUST be 1ms (not 10ms). The bootloader window opens within
/// ~8ms after the final phase, so rapid polling is required to catch the ACK.
fn get_bus(port: &mut dyn serialport::SerialPort) -> Result<bool> {
    // CRITICAL: 1ms read timeout — the bootloader window is narrow (~8ms after Phase 5).
    // With 10ms timeout each link_check_once wasted too much time and missed the window.
    let _ = port.set_timeout(Duration::from_millis(1));

    // Initialize to known state (serialport crate may start with DTR/RTS in unexpected state)
    let _ = port.write_data_terminal_ready(false);
    let _ = port.write_request_to_send(false);
    std::thread::sleep(Duration::from_millis(50));

    for _attempt in 0..30u32 {
        // 5-phase DTR/RTS pulse pattern
        let _ = port.write_data_terminal_ready(true);
        let _ = port.write_request_to_send(true);
        std::thread::sleep(Duration::from_millis(50));

        let _ = port.write_data_terminal_ready(false);
        let _ = port.write_request_to_send(false);
        std::thread::sleep(Duration::from_millis(20));

        let _ = port.write_data_terminal_ready(true);
        let _ = port.write_request_to_send(false);
        std::thread::sleep(Duration::from_millis(50));

        // Phase 4: DTR=0 (release reset), RTS=1 (bootloader mode) — device boots into bootloader
        let _ = port.write_data_terminal_ready(false);
        let _ = port.write_request_to_send(true);
        std::thread::sleep(Duration::from_millis(50));

        let _ = port.write_data_terminal_ready(false);
        let _ = port.write_request_to_send(false);
        // No sleep here — immediately start link checks after Phase 5

        // Try LinkCheck rapidly — bootloader window opens within ~8ms of Phase 5
        for _ in 0..200 {
            if link_check_once(port) {
                let _ = port.set_timeout(Duration::from_millis(200));
                return Ok(true);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let _ = port.set_timeout(Duration::from_millis(200));
    Ok(false)
}

/// Ask the device to switch baud rate to `baud` after `delay_ms` delay, then
/// switch the host side and verify with the device's ACK at the new baud rate.
///
/// IMPORTANT: must drain input buffer before sending (stale link check ACKs from
/// get_bus() loop will interfere). Matches py3's _drain_input() + set_baudrate().
fn set_baud_rate(
    port: &mut dyn serialport::SerialPort,
    baud: u32,
    delay_ms: u64,
) -> Result<bool> {
    let [b0, b1, b2, b3] = baud.to_le_bytes();
    let d = delay_ms as u8;
    // Short CMD=0x0F, LEN=6: [CMD, BR0..3, DELAY]
    let tx = [0x01u8, 0xE0, 0xFC, 0x06, 0x0F, b0, b1, b2, b3, d];

    // Drain stale link check ACKs from the get_bus() loop (matches py3's _drain_input())
    let _ = port.clear(serialport::ClearBuffer::Input);
    std::thread::sleep(Duration::from_millis(50));  // let any in-flight ACKs arrive
    let _ = port.clear(serialport::ClearBuffer::Input);

    port.write_all(&tx)?;
    port.flush()?;
    // Wait for TX to fully drain before switching baud (py3: while out_waiting > 0)
    std::thread::sleep(Duration::from_millis(10));
    // Wait half the device's delay, then switch host baud
    std::thread::sleep(Duration::from_millis(delay_ms / 2));
    port.set_baud_rate(baud).context("Failed to change host baud rate")?;
    // Read ACK at new baud: [0x04, 0x0E, 0x05, 0x01, 0xE0, 0xFC, 0x0F, 0x00]
    // py3 _ensure_short_response checks: buf[0:2]=04 0E, buf[3:6]=01 E0 FC, buf[6]=CMD(0x0F)
    let mut buf = [0u8; 8];
    if read_exact_timeout(port, &mut buf, Duration::from_millis(600)).is_err() {
        return Ok(false);
    }
    let ok = buf[..2] == [0x04, 0x0E]
          && buf[3..6] == [0x01, 0xE0, 0xFC]
          && buf[6] == 0x0F;
    Ok(ok)
}

/// Read flash JEDEC ID (Manufacturer, Memory Type, Capacity).
/// Returns 3-byte value: manufacturer | (type<<8) | (capacity<<16).
fn get_flash_mid(port: &mut dyn serialport::SerialPort) -> Result<u32> {
    // Long CMD=0x0E, LEN=5: [CMD, 0x9F, 0x00, 0x00, 0x00]
    let tx = [0x01u8, 0xE0, 0xFC, 0xFF, 0xF4, 0x05, 0x00, 0x0E, 0x9F, 0x00, 0x00, 0x00];
    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;
    // RX: 15 bytes (9-byte long header + 6-byte body)
    let mut buf = [0u8; 15];
    read_exact_timeout(port, &mut buf, Duration::from_secs(3))?;
    if buf[0] != 0x04 || buf[1] != 0x0E || buf[2] != 0xFF {
        bail!("GetFlashMID: bad response prefix {:02x?}", &buf[..4]);
    }
    // MID at buf[12..14]: (ToInt32(buf,11) >> 8) = buf[12]|(buf[13]<<8)|(buf[14]<<16)
    let mid = buf[12] as u32 | ((buf[13] as u32) << 8) | ((buf[14] as u32) << 16);
    Ok(mid)
}

/// Read one flash Status Register byte. `sr_cmd` is the SPI read-SR command
/// (0x05 = RDSR1, 0x35 = RDSR2, 0x15 = RDSR3).
fn read_flash_sr(port: &mut dyn serialport::SerialPort, sr_cmd: u8) -> Result<u8> {
    // Long CMD=0x0C, LEN=2: [CMD, sr_cmd]
    let tx = [0x01u8, 0xE0, 0xFC, 0xFF, 0xF4, 0x02, 0x00, 0x0C, sr_cmd];
    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;
    // RX: 13 bytes (9-byte long header + 4-byte body)
    let mut buf = [0u8; 13];
    read_exact_timeout(port, &mut buf, Duration::from_secs(2))?;
    if buf[0] != 0x04 || buf[1] != 0x0E || buf[2] != 0xFF {
        bail!("ReadFlashSR 0x{:02x}: bad response prefix", sr_cmd);
    }
    // SR value at buf[11]
    Ok(buf[11])
}

/// Write 1-byte flash Status Register. `wr_cmd` is typically 0x01 (WRSR).
fn write_flash_sr_1(
    port: &mut dyn serialport::SerialPort,
    wr_cmd: u8,
    val: u8,
) -> Result<()> {
    // Long CMD=0x0D, LEN=3: [CMD, wr_cmd, val]
    let tx = [0x01u8, 0xE0, 0xFC, 0xFF, 0xF4, 0x03, 0x00, 0x0D, wr_cmd, val];
    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;
    let mut buf = [0u8; 13];
    read_exact_timeout(port, &mut buf, Duration::from_secs(2))?;
    if buf[0] != 0x04 || buf[1] != 0x0E || buf[2] != 0xFF {
        bail!("WriteFlashSR1: bad response prefix");
    }
    Ok(())
}

/// Write 2-byte flash Status Register (SR1+SR2 simultaneously). `wr_cmd` = 0x01.
fn write_flash_sr_2(
    port: &mut dyn serialport::SerialPort,
    wr_cmd: u8,
    val: u16,
) -> Result<()> {
    // Long CMD=0x0D, LEN=4: [CMD, wr_cmd, val_lo, val_hi]
    let [v0, v1] = val.to_le_bytes();
    let tx = [0x01u8, 0xE0, 0xFC, 0xFF, 0xF4, 0x04, 0x00, 0x0D, wr_cmd, v0, v1];
    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;
    let mut buf = [0u8; 14];
    read_exact_timeout(port, &mut buf, Duration::from_secs(2))?;
    if buf[0] != 0x04 || buf[1] != 0x0E || buf[2] != 0xFF {
        bail!("WriteFlashSR2: bad response prefix");
    }
    Ok(())
}

/// Erase one sector/block. `sz_cmd`: 0x20 = 4K sector, 0xD8 = 64K block.
/// Returns true on ACK, false on timeout/bad response.
fn erase_sector(
    port: &mut dyn serialport::SerialPort,
    addr: u32,
    sz_cmd: u8,
) -> Result<bool> {
    // Long CMD=0x0F, LEN=6: [CMD, sz_cmd, A0..A3]
    let [a0, a1, a2, a3] = addr.to_le_bytes();
    let tx = [0x01u8, 0xE0, 0xFC, 0xFF, 0xF4, 0x06, 0x00, 0x0F, sz_cmd, a0, a1, a2, a3];
    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;
    // RX: 16 bytes (9-byte long header + 7-byte body)
    // Timeout: 64K block erases can take up to ~5 s on slow flash
    let timeout = if sz_cmd == 0xD8 { Duration::from_secs(8) } else { Duration::from_secs(3) };
    let mut buf = [0u8; 16];
    match read_exact_timeout(port, &mut buf, timeout) {
        Ok(_) => {}
        Err(e) => {
            log::warn!("[BK] Erase timeout at 0x{:08x} (sz=0x{:02x}): {}", addr, sz_cmd, e);
            return Ok(false);
        }
    }
    let ok = buf[0] == 0x04 && buf[1] == 0x0E && buf[2] == 0xFF;
    if !ok {
        log::warn!("[BK] Erase bad response at 0x{:08x}: {:02x?}", addr, &buf[..5]);
    }
    Ok(ok)
}

/// Write one 4096-byte sector. Returns true on ACK with correct address echo.
fn write_sector_4k(
    port: &mut dyn serialport::SerialPort,
    addr: u32,
    data: &[u8; SECTOR_SIZE],
) -> Result<bool> {
    // Long CMD=0x07, LEN=4101 (0x1005): [CMD, A0..A3, DATA[4096]]
    let [a0, a1, a2, a3] = addr.to_le_bytes();
    let [l0, l1] = 4101u16.to_le_bytes(); // 0x05, 0x10
    let mut tx = Vec::with_capacity(4108);
    tx.extend_from_slice(&[0x01, 0xE0, 0xFC, 0xFF, 0xF4, l0, l1, 0x07, a0, a1, a2, a3]);
    tx.extend_from_slice(data.as_slice());

    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;

    // RX: 15 bytes (9-byte long header + 6-byte body: [CMD, extra, A0..A3])
    let mut buf = [0u8; 15];
    match read_exact_timeout(port, &mut buf, Duration::from_secs(5)) {
        Ok(_) => {}
        Err(e) => {
            log::warn!("[BK] Write timeout at 0x{:08x}: {}", addr, e);
            return Ok(false);
        }
    }
    if buf[0] != 0x04 || buf[1] != 0x0E || buf[2] != 0xFF {
        log::warn!("[BK] Write bad response at 0x{:08x}: {:02x?}", addr, &buf[..5]);
        return Ok(false);
    }
    // buf[9] = cmd echo (0x07), buf[11..14] = address echo (LE)
    let echo = u32::from_le_bytes([buf[11], buf[12], buf[13], buf[14]]);
    if echo != addr {
        log::warn!("[BK] Write addr mismatch: sent 0x{:08x} got echo 0x{:08x}", addr, echo);
        return Ok(false);
    }
    Ok(true)
}

/// CRC check for a flash range [start_addr, end_addr] inclusive.
/// Returns the 32-bit CRC reported by the bootloader.
/// Short CMD=0x10, payload = [start_addr(4 LE), end_addr(4 LE)]
fn check_crc(
    port: &mut dyn serialport::SerialPort,
    start_addr: u32,
    end_addr: u32,
) -> Result<u32> {
    let [s0, s1, s2, s3] = start_addr.to_le_bytes();
    let [e0, e1, e2, e3] = end_addr.to_le_bytes();
    // Short packet: LEN=9 (1 CMD + 8 payload)
    let tx = [0x01u8, 0xE0, 0xFC, 0x09, 0x10, s0, s1, s2, s3, e0, e1, e2, e3];
    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;
    // Response: at least 11 bytes short header + CRC at bytes 7..10
    // [0x04, 0x0E, LEN, 0x01, 0xE0, 0xFC, 0x10, CRC0, CRC1, CRC2, CRC3]
    let mut buf = [0u8; 11];
    read_exact_timeout(port, &mut buf, Duration::from_secs(10))?;
    if buf[0] != 0x04 || buf[1] != 0x0E || buf[6] != 0x10 {
        bail!("CheckCRC: bad response {:02x?}", &buf);
    }
    Ok(u32::from_le_bytes([buf[7], buf[8], buf[9], buf[10]]))
}

/// Read 4K sector from flash at `addr`. Returns the 4096 data bytes.
/// Long CMD=0x09, payload = [addr(4 LE)]
fn read_sector_4k(
    port: &mut dyn serialport::SerialPort,
    addr: u32,
) -> Result<Vec<u8>> {
    let [a0, a1, a2, a3] = addr.to_le_bytes();
    // Long packet: LEN=5 (1 CMD + 4 addr)
    let tx = [0x01u8, 0xE0, 0xFC, 0xFF, 0xF4, 0x05, 0x00, 0x09, a0, a1, a2, a3];
    let _ = port.clear(serialport::ClearBuffer::Input);
    port.write_all(&tx)?;
    port.flush()?;
    // Response: 9-byte long header + 4K data
    // [0x04, 0x0E, 0xFF, 0x01, 0xE0, 0xFC, 0xF4, LEN_LO, LEN_HI, CMD, data...]
    let mut header = [0u8; 9];
    read_exact_timeout(port, &mut header, Duration::from_secs(5))?;
    if header[0] != 0x04 || header[1] != 0x0E || header[2] != 0xFF {
        bail!("ReadSector: bad response header {:02x?}", &header);
    }
    let body_len = u16::from_le_bytes([header[7], header[8]]) as usize;
    // body_len includes CMD byte, so data is body_len - 1 bytes (but may include addr)
    // In practice this returns the data directly; read body_len bytes total
    let mut body = vec![0u8; body_len];
    read_exact_timeout(port, &mut body, Duration::from_secs(5))?;
    // body[0] = CMD echo (0x09), body[1..5] = addr, body[5..] = data
    if body_len < SECTOR_SIZE + 5 {
        bail!("ReadSector: short body {}/{}", body_len, SECTOR_SIZE + 5);
    }
    Ok(body[5..5+SECTOR_SIZE].to_vec())
}

// ─── Flash unprotect ──────────────────────────────────────────────────────────

/// Look up SR parameters for a given Flash MID.
/// Returns `(sz_sr, [rd_cmd_sr1, rd_cmd_sr2], cw_msk_16bit)`.
fn flash_sr_params(mid: u32) -> (usize, [u8; 2], u32) {
    // Mask meanings (16-bit SR view: SR2<<8 | SR1):
    //   Bits 2-6  (SR1): BP0-BP4 write-protect = 0x007C
    //   Bit  14   (SR2 bit 6): CMP complement-protect = 0x4000
    const M2: u32 = 0x407C; // 2-byte write: clear CMP + BP0-BP4
    const M1: u32 = 0x007C; // 1-byte write: clear BP0-BP4 only

    match mid {
        // 1-byte SR chips (GD25Dxx series, small XTX)
        0x144051 | 0x134051 | 0x14405E | 0x13405E | 0x13311C => (1, [0x05, 0xFF], M1),
        // GD25WD80E (1-byte SR despite having SR2)
        0x1464C8 => (1, [0x05, 0xFF], M1),
        // ESMT EN25QH16B – 1-byte SR
        0x15701C => (1, [0x05, 0xFF], 0x003C),
        // MXIC MX25V variants – special mask
        0x1423C2 | 0x1523C2 => (2, [0x05, 0x15], 0x3012),
        // GT25Q16B – 3-register chip, treat as 2-byte write
        0x1560C4 => (2, [0x05, 0x35], M2),
        // All other chips (GD25Q, XTX25F/Q, Puya25Q, WB25Q, XM25QU, TH25Q, ESMT, DSZB…)
        _ => {
            // Heuristic: flash ≥ 8 MB (capacity code ≥ 0x14) → 2-byte SR
            let cap = (mid >> 16) & 0xFF;
            if cap >= 0x14 { (2, [0x05, 0x35], M2) } else { (1, [0x05, 0x35], M1) }
        }
    }
}

/// Clear write-protection bits in the flash Status Register(s).
fn unprotect_flash(port: &mut dyn serialport::SerialPort, mid: u32) -> Result<()> {
    let (sz_sr, rd_cmds, cw_msk) = flash_sr_params(mid);
    let sr1 = read_flash_sr(port, rd_cmds[0])?;
    let sr_val: u16 = if sz_sr >= 2 && rd_cmds[1] != 0xFF {
        let sr2 = read_flash_sr(port, rd_cmds[1]).unwrap_or(0);
        ((sr2 as u16) << 8) | (sr1 as u16)
    } else {
        sr1 as u16
    };
    let new_val = sr_val & !(cw_msk as u16);
    if new_val == sr_val {
        log::info!("[BK] Flash already unprotected (SR=0x{:04x})", sr_val);
        return Ok(());
    }
    log::info!("[BK] Unprotect: SR 0x{:04x} → 0x{:04x}", sr_val, new_val);
    if sz_sr >= 2 && rd_cmds[1] != 0xFF {
        write_flash_sr_2(port, 0x01, new_val)?;
    } else {
        write_flash_sr_1(port, 0x01, new_val as u8)?;
    }
    std::thread::sleep(Duration::from_millis(20)); // SR write time
    Ok(())
}

// ─── Erase + write primitives ─────────────────────────────────────────────────

/// Erase `num_sectors` × 4 K sectors starting at `start_addr`.
/// Uses 64 K block erases where possible (same as BK7231GUIFlashTool eraseRange).
/// `progress_cb(done, total)` is called after each erase unit; return false to cancel.
fn erase_range<F>(
    port: &mut dyn serialport::SerialPort,
    start_addr: u32,
    num_sectors: usize,
    mut progress_cb: F,
) -> Result<()>
where
    F: FnMut(usize, usize) -> bool,
{
    let mut current = start_addr as usize / SECTOR_SIZE;
    let end = current + num_sectors;
    let mut done = 0usize;

    macro_rules! do_erase_4k {
        () => {{
            let addr = (current * SECTOR_SIZE) as u32;
            let mut ok = false;
            for _ in 0..5 {
                if erase_sector(port, addr, 0x20)? { ok = true; break; }
                std::thread::sleep(Duration::from_millis(50));
            }
            if !ok { bail!("4K erase failed at 0x{:08x} after 5 retries", addr); }
            current += 1;
            done += 1;
            if !progress_cb(done.min(num_sectors), num_sectors) { return Ok(()); }
        }};
    }

    // 1. Align to 64 K block boundary with 4 K erases
    while current < end && (current % SECTORS_PER_BLOCK) != 0 {
        do_erase_4k!();
    }
    // 2. 64 K block erases
    while end - current >= SECTORS_PER_BLOCK {
        let addr = (current * SECTOR_SIZE) as u32;
        let mut ok = false;
        for _ in 0..5 {
            if erase_sector(port, addr, 0xD8)? { ok = true; break; }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !ok { bail!("64K erase failed at 0x{:08x} after 5 retries", addr); }
        current += SECTORS_PER_BLOCK;
        done += SECTORS_PER_BLOCK;
        if !progress_cb(done.min(num_sectors), num_sectors) { return Ok(()); }
    }
    // 3. Remaining 4 K erases
    while current < end {
        do_erase_4k!();
    }
    Ok(())
}

/// Erase then write `data` to flash at `start_addr`, emitting Tauri progress events.
/// Progress ranges from `pct_start` to `pct_end`.
fn flash_data(
    port: &mut dyn serialport::SerialPort,
    app: &AppHandle,
    data: &[u8],
    start_addr: u32,
    pct_start: f32,
    pct_end: f32,
    label: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let num_sectors = (data.len() + SECTOR_SIZE - 1) / SECTOR_SIZE;
    let erase_end = pct_start + (pct_end - pct_start) * 0.4;

    // ── Erase phase ──────────────────────────────────────────────────────────
    erase_range(port, start_addr, num_sectors, |done, total| {
        if cancel.load(Ordering::Relaxed) { return false; }
        let pct = pct_start + (erase_end - pct_start) * (done as f32 / total as f32);
        app.emit("flash:progress", FlashEvent::info(
            "Erasing", pct,
            &format!("{label} – erasing {done}/{total}")
        )).ok();
        true
    })?;

    if cancel.load(Ordering::Relaxed) { bail!("Flash cancelled by user"); }
    app.emit("flash:progress", FlashEvent::info(
        "Writing", erase_end, &format!("{label} – erase done, writing…")
    )).ok();

    // ── Write phase ───────────────────────────────────────────────────────────
    for i in 0..num_sectors {
        if cancel.load(Ordering::Relaxed) { bail!("Flash cancelled by user"); }

        let offset = i * SECTOR_SIZE;
        let end_off = (offset + SECTOR_SIZE).min(data.len());
        let addr = start_addr + offset as u32;

        let mut sector = [0xFFu8; SECTOR_SIZE];
        sector[..end_off - offset].copy_from_slice(&data[offset..end_off]);

        let mut ok = false;
        for attempt in 0..3 {
            match write_sector_4k(port, addr, &sector)? {
                true => { ok = true; break; }
                false => {
                    log::warn!("[BK] Retry write sector {} attempt {}", i, attempt + 1);
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        if !ok { bail!("Write failed at 0x{:08x} after 3 retries", addr); }

        let pct = erase_end + (pct_end - erase_end) * ((i + 1) as f32 / num_sectors as f32);
        app.emit("flash:progress", FlashEvent::info(
            "Writing", pct,
            &format!("{label} – writing {}/{num_sectors}", i + 1)
        )).ok();
    }
    Ok(())
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Read and parse info.json from a .soc zip archive.
pub fn read_soc_info(soc_path: &str) -> Result<SocInfo> {
    let file = std::fs::File::open(soc_path)
        .with_context(|| format!("Cannot open .soc: {soc_path}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .context("Not a valid .soc file (expected zip)")?;
    let entry = archive.by_name("info.json")
        .context("info.json not found inside .soc")?;
    let info: SocInfo = serde_json::from_reader(entry)
        .context("Failed to parse info.json")?;
    Ok(info)
}

// ─── Subprocess flash (BK7258 / Air8101) ─────────────────────────────────────
//
// air602_flash.exe is bundled inside the .soc archive.  It handles the ROM
// bootloader protocol (link-check, baud-switch, erase, write, Boot_Reboot).
// After the process exits it has already sent the Boot_Reboot command which
// triggers a power-on reset; the device boots LuatOS ~230 ms later.
//
// Critical timing: we must open the log serial port IMMEDIATELY after the
// subprocess exits — any delay > ~10 s means the boot window is missed and
// Windows discards all buffered data.  We therefore:
//   1. Spawn subprocess with piped stdout (background thread drains it).
//   2. Call child.wait() — returns the instant the process exits.
//   3. Open log port in the same thread, before tokio regains control.
//   4. Read for LOG_CAPTURE_SECS, emitting each line as a flash:log event.

const LOG_CAPTURE_SECS: u64 = 20;

/// Run air602_flash.exe as a subprocess, then immediately capture the device
/// boot log.  Emits `flash:progress` and `flash:log` events.
/// Returns the captured boot log as individual text lines.
fn flash_via_subprocess(
    app: &AppHandle,
    exe_path: &std::path::Path,
    rom_path: &std::path::Path,
    port: &str,
    log_br: u32,
    cancel: &AtomicBool,
) -> Result<Vec<String>> {
    macro_rules! emit {
        ($evt:expr) => { let _ = app.emit("flash:progress", $evt); }
    }

    // Strip "COM" prefix — air602_flash.exe wants a bare number
    let port_num: String = port.chars().filter(|c| c.is_ascii_digit()).collect();
    if port_num.is_empty() {
        bail!("Invalid port name: {port}");
    }

    emit!(FlashEvent::info("Flashing", 5.0,
        &format!("Starting air602_flash.exe on {port} (~37 s)…")));

    // Spawn with piped stdout; stderr discarded so exe never blocks on write
    let mut child = std::process::Command::new(exe_path)
        .args(["download", "-p", &port_num, "-b", "2000000", "-s", "0", "-i"])
        .arg(rom_path)
        .current_dir(exe_path.parent().unwrap_or(std::path::Path::new(".")))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to launch air602_flash.exe")?;

    // Background thread: drain subprocess stdout and emit log/progress events
    let app_clone = app.clone();
    let stdout = child.stdout.take().expect("stdout piped");
    let progress_thread = std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let mut pct = 5.0f32;
        for line in BufReader::new(stdout).lines().filter_map(|l| l.ok()) {
            let t = line.trim();
            if t.is_empty() { continue; }
            if t.contains("Gotten Bus") || t.contains("Gotten bus") { pct = 15.0; }
            else if t.contains("baudrate") { pct = 22.0; }
            else if t.contains("Boot_Reboot") { pct = 90.0; }
            else if t.contains("All Finished") { pct = 93.0; }
            else if t.contains("Writing") { pct = (pct + 0.4).min(88.0); }
            else if t.contains("Failed") || t.contains("Error") { pct = pct; }
            let _ = app_clone.emit("flash:progress",
                FlashEvent::info("Flashing", pct, &format!("[exe] {t}")));
        }
    });

    // Wait for subprocess to exit — returns the moment the process terminates,
    // independent of whether the pipe has been fully drained yet.
    let _status = child.wait().context("air602_flash.exe wait() failed")?;

    emit!(FlashEvent::info("Booting", 94.0,
        &format!("Firmware sent! Opening {port} @ {log_br} to capture boot log…")));

    // Open log port immediately — device reboots ~230 ms after exe exit
    let mut log_port = {
        let mut last_err = String::new();
        let mut port_opt = None;
        // Retry briefly in case Windows COM driver hasn't fully released yet
        for _ in 0..10 {
            match serialport::new(port, log_br)
                .timeout(Duration::from_millis(200))
                .open()
            {
                Ok(p) => { port_opt = Some(p); break; }
                Err(e) => {
                    last_err = e.to_string();
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        match port_opt {
            Some(p) => p,
            None => {
                let _ = progress_thread.join();
                emit!(FlashEvent::done_err(&format!("Cannot open log port {port}: {last_err}")));
                bail!("Cannot open log port {port}: {last_err}");
            }
        }
    };

    // Capture boot log for LOG_CAPTURE_SECS seconds
    let mut log_bytes: Vec<u8> = Vec::new();
    let mut line_buf: Vec<u8> = Vec::new();
    let mut read_buf = [0u8; 512];
    let deadline = Instant::now() + Duration::from_secs(LOG_CAPTURE_SECS);

    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            emit!(FlashEvent::log_line("[INFO] Boot log capture cancelled"));
            break;
        }
        match log_port.read(&mut read_buf) {
            Ok(n) if n > 0 => {
                log_bytes.extend_from_slice(&read_buf[..n]);
                for &b in &read_buf[..n] {
                    if b == b'\n' {
                        let s = String::from_utf8_lossy(&line_buf)
                            .trim_end_matches('\r').to_string();
                        line_buf.clear();
                        if !s.is_empty() {
                            let _ = app.emit("flash:log", &s);
                        }
                    } else {
                        line_buf.push(b);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            _ => {}
        }
    }
    drop(log_port);
    let _ = progress_thread.join();

    // Parse lines and determine PASS/FAIL
    let log_text = String::from_utf8_lossy(&log_bytes);
    let lines: Vec<String> = log_text
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let boot_kw = ["luat:", "ap0:", "ap1:", "LuatOS", "EasyFlash"];
    let passed = boot_kw.iter().any(|kw| log_text.contains(kw));

    if passed {
        emit!(FlashEvent::done_ok(&format!(
            "PASS — firmware booted ({} log lines, {} bytes)",
            lines.len(), log_bytes.len()
        )));
    } else {
        emit!(FlashEvent::done_err(&format!(
            "FAIL — no boot keywords found in {} bytes of log output",
            log_bytes.len()
        )));
    }

    Ok(lines)
}

/// Full BK7258 (Air8101) flash routine. Must be called from a blocking context.
///
/// If `air602_flash.exe` is present in the .soc archive (BK7258 firmware), the
/// subprocess approach is used: the exe handles the ROM bootloader protocol and
/// sends Boot_Reboot at the end, then the device is allowed to boot and its log
/// output is captured for `LOG_CAPTURE_SECS` seconds.
///
/// If the exe is absent (non-BK7258 .soc), the native Rust serial protocol is
/// used to flash firmware and optionally a script partition.
///
/// Returns the captured boot log lines (empty if native-Rust path was taken).
pub fn flash_bk7258(
    app: AppHandle,
    _flash_child: Arc<Mutex<Option<std::process::Child>>>,
    soc_path: &str,
    script_folder: Option<&str>,
    port: &str,
    baud_rate: Option<u32>,
    cancel: Arc<AtomicBool>,
) -> Result<Vec<String>> {
    cancel.store(false, Ordering::Relaxed);

    macro_rules! emit {
        ($evt:expr) => { let _ = app.emit("flash:progress", $evt); }
    }
    macro_rules! check_cancel {
        () => {
            if cancel.load(Ordering::Relaxed) { bail!("Flash cancelled by user"); }
        };
    }

    emit!(FlashEvent::info("Preparing", 1.0, "Extracting .soc…"));

    // ── 1. Extract .soc ───────────────────────────────────────────────────────
    let tempdir = tempfile::tempdir().context("Failed to create temp dir")?;
    {
        let file = std::fs::File::open(soc_path)
            .with_context(|| format!("Cannot open {soc_path}"))?;
        let mut archive = zip::ZipArchive::new(file)?;
        archive.extract(tempdir.path()).context("Extraction failed")?;
    }

    // ── 2. Parse info.json ────────────────────────────────────────────────────
    let info: SocInfo = serde_json::from_reader(
        std::fs::File::open(tempdir.path().join("info.json"))
            .context("info.json missing")?
    ).context("Parse info.json")?;

    let log_br: u32 = info.user.as_ref()
        .and_then(|u| u.log_br.as_deref())
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);

    let rom_path = tempdir.path().join(&info.rom.file);
    if !rom_path.exists() {
        bail!("ROM file '{}' not found in .soc", info.rom.file);
    }

    // ── 3. Subprocess path (preferred): use bundled air602_flash.exe ──────────
    let exe_path = tempdir.path().join("air602_flash.exe");
    if exe_path.exists() {
        if script_folder.is_some() {
            emit!(FlashEvent::log_line(
                "[WARN] Script flashing is not supported with the subprocess approach; \
                 only firmware will be flashed."
            ));
        }
        emit!(FlashEvent::info("Preparing", 3.0,
            &format!("Firmware: {} (subprocess mode)", info.rom.file)));
        return flash_via_subprocess(&app, &exe_path, &rom_path, port, log_br, &cancel);
    }

    // ── 4. Native Rust path (fallback for non-BK7258 or legacy .soc) ─────────
    let flash_br: u32 = info.download.force_br.as_deref()
        .and_then(|s| s.parse().ok())
        .or(baud_rate)
        .unwrap_or(2_000_000);

    let bl_addr = parse_addr(info.download.bl_addr.as_deref().unwrap_or("0")).unwrap_or(0) as u32;
    let rom_data = std::fs::read(&rom_path)
        .with_context(|| format!("Cannot read {}", info.rom.file))?;
    emit!(FlashEvent::info("Preparing", 2.0,
        &format!("Firmware: {} bytes @ 0x{:x} (native mode)", rom_data.len(), bl_addr)));

    // Optionally build script.bin
    let script_payload: Option<(Vec<u8>, u32)> = if let Some(folder) = script_folder {
        let fp = Path::new(folder);
        if fp.is_dir() {
            emit!(FlashEvent::info("Building", 3.0, "Building script.bin from Lua files…"));
            match build_script_bin(fp, &info, tempdir.path()) {
                Ok(sp) => {
                    let d = std::fs::read(&sp)?;
                    let sa = parse_addr(
                        info.download.script_addr.as_deref().unwrap_or("0x200000")
                    ).unwrap_or(0x200000) as u32;
                    emit!(FlashEvent::info("Building", 5.0,
                        &format!("Script: {} bytes @ 0x{:x}", d.len(), sa)));
                    Some((d, sa))
                }
                Err(e) => {
                    emit!(FlashEvent::log_line(
                        &format!("[WARN] Script build failed: {e} — flashing firmware only")));
                    None
                }
            }
        } else { None }
    } else { None };

    check_cancel!();

    emit!(FlashEvent::info("Connecting", 6.0, &format!("Opening {port} @ 115200…")));
    let mut serial = serialport::new(port, 115_200)
        .timeout(Duration::from_millis(200))
        .open()
        .with_context(|| format!("Cannot open serial port {port}"))?;

    emit!(FlashEvent::info("Connecting", 8.0, "Resetting device into bootloader…"));
    if !get_bus(&mut *serial)? {
        bail!("Cannot enter bootloader mode on {port}. \
               Check the cable and that DTR/RTS are wired.");
    }
    emit!(FlashEvent::info("Connecting", 10.0, "Bootloader link established!"));

    if flash_br != 115_200 {
        emit!(FlashEvent::info("Connecting", 11.0,
            &format!("Switching to {flash_br} bps…")));
        match set_baud_rate(&mut *serial, flash_br, 200) {
            Ok(true)  => { emit!(FlashEvent::info("Connecting", 12.0,
                &format!("Baud rate set to {flash_br}"))); }
            Ok(false) => { emit!(FlashEvent::log_line(
                "[WARN] Baud rate switch ACK failed — continuing at 115200")); }
            Err(e)    => { emit!(FlashEvent::log_line(
                &format!("[WARN] set_baud_rate error: {e}"))); }
        }
    }

    check_cancel!();

    let mid = match get_flash_mid(&mut *serial) {
        Ok(m) => { emit!(FlashEvent::info("Connecting", 13.0,
            &format!("Flash MID: 0x{m:06x}"))); m }
        Err(e) => { emit!(FlashEvent::log_line(&format!("[WARN] GetFlashMID: {e}"))); 0 }
    };

    emit!(FlashEvent::info("Connecting", 14.0, "Unprotecting flash…"));
    if let Err(e) = unprotect_flash(&mut *serial, mid) {
        emit!(FlashEvent::log_line(&format!("[WARN] Unprotect: {e}")));
    } else {
        emit!(FlashEvent::info("Connecting", 15.0, "Flash unprotected"));
    }

    check_cancel!();

    let fw_end_pct = if script_payload.is_some() { 80.0f32 } else { 98.0f32 };
    flash_data(&mut *serial, &app, &rom_data, bl_addr,
               15.0, fw_end_pct, "Firmware", &cancel)?;

    check_cancel!();

    if let Some((script_data, script_addr)) = script_payload {
        flash_data(&mut *serial, &app, &script_data, script_addr,
                   80.0, 98.0, "Script", &cancel)?;
    }

    drop(serial);
    emit!(FlashEvent::done_ok("Flash complete! Device is rebooting."));
    Ok(vec![])  // native path: no boot log captured
}

// ─── Script synthesis ─────────────────────────────────────────────────────────

fn build_script_bin(folder: &Path, info: &SocInfo, out_dir: &Path) -> Result<PathBuf> {
    let bitw = info.script.bitw.unwrap_or(64);
    let use_luac = info.script.use_luac.unwrap_or(true);
    let use_bkcrc = info.rom.fs.as_ref()
        .and_then(|fs| fs.script.as_ref())
        .and_then(|s| s.bkcrc)
        .unwrap_or(false);

    let mut entries: Vec<crate::luadb::LuadbEntry> = Vec::new();

    for entry in std::fs::read_dir(folder).context("Cannot read script folder")? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lua") { continue; }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read(&path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        if use_luac {
            match compile_lua(&src, &name, bitw) {
                Ok(bc) => {
                    let luac_name = format!("{}.luac", &name[..name.len()-4]);
                    entries.push(crate::luadb::LuadbEntry { filename: luac_name, data: bc });
                    continue;
                }
                Err(e) => log::warn!("luac failed for {name}: {e} — using raw lua"),
            }
        }
        entries.push(crate::luadb::LuadbEntry { filename: name, data: src });
    }

    if entries.is_empty() {
        bail!("No .lua files found in {}", folder.display());
    }

    let mut data = crate::luadb::pack_luadb(&entries);
    if use_bkcrc {
        data = crate::luadb::add_bk_crc(&data);
    }

    let out = out_dir.join("script_with_crc.bin");
    std::fs::write(&out, &data)?;
    log::info!("[BK] script_with_crc.bin: {} bytes", data.len());
    Ok(out)
}

fn compile_lua(src: &[u8], name: &str, bitw: u32) -> Result<Vec<u8>> {
    let candidates: &[&str] = if bitw == 64 {
        &[
            r"D:\github\luatools_py3\_temp\tools\luac_64bit.exe",
            r"D:\github\luatools_py3\_temp\tools\luac.exe",
        ]
    } else {
        &[r"D:\github\luatools_py3\_temp\tools\luac.exe"]
    };
    let luac = candidates.iter().map(Path::new).find(|p| p.exists())
        .ok_or_else(|| anyhow::anyhow!("luac_64bit.exe not found"))?;
    let tmp_src = tempfile::Builder::new().suffix(".lua").tempfile()?;
    let tmp_out = tempfile::Builder::new().suffix(".luac").tempfile()?;
    std::fs::write(tmp_src.path(), src)?;
    let out = Command::new(luac)
        .arg("-o").arg(tmp_out.path())
        .arg(tmp_src.path())
        .output()
        .with_context(|| format!("Failed to run {}", luac.display()))?;
    if !out.status.success() {
        bail!("luac error for {name}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(std::fs::read(tmp_out.path())?)
}

// ─── Address parsing ──────────────────────────────────────────────────────────

/// Parse a flash address from info.json (may be "0x…" hex or bare hex digits).
pub fn parse_addr(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok();
    }
    u64::from_str_radix(s, 16).ok()
}

// ─── Integration tests (require Air8101 on COM6) ──────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Diagnostic: open COM6, try one LinkCheck, print raw bytes received.
    /// Run with: cargo test bk_diag -- --ignored --nocapture
    #[test]
    #[ignore = "diagnostic — requires Air8101 connected on COM6 already in bootloader"]
    fn bk_diag() {
        let port_name = "COM6";
        let mut serial = serialport::new(port_name, 115_200)
            .timeout(Duration::from_millis(500))
            .open()
            .expect("open COM6");
        println!("\n=== BK7231 raw diagnostic on {port_name} ===");

        // Try LinkCheck once (device must already be in bootloader mode)
        let tx = [0x01u8, 0xE0, 0xFC, 0x01, 0x00];
        println!("Sending LinkCheck: {:02x?}", &tx);
        serial.write_all(&tx).unwrap();
        serial.flush().unwrap();

        let mut buf = [0u8; 32];
        let mut n = 0;
        let deadline = Instant::now() + Duration::from_millis(1000);
        while n < 8 && Instant::now() < deadline {
            match serial.read(&mut buf[n..]) {
                Ok(k) if k > 0 => { n += k; println!("  read {} bytes so far: {:02x?}", n, &buf[..n]); }
                _ => {}
            }
        }
        if n == 0 {
            println!("  NO RESPONSE (device may not be in bootloader mode)");
        } else {
            println!("Final rx {} bytes: {:02x?}", n, &buf[..n]);
        }

        // Also check what's already in the buffer (device may be running firmware)
        println!("Testing DTR/RTS toggle — watching for response after reset:");
        let _ = serial.write_data_terminal_ready(true);
        let _ = serial.write_request_to_send(true);
        std::thread::sleep(Duration::from_millis(100));
        let _ = serial.write_data_terminal_ready(false);
        let _ = serial.write_request_to_send(false);
        std::thread::sleep(Duration::from_millis(20));

        // Send LinkCheck immediately
        let _ = serial.clear(serialport::ClearBuffer::Input);
        serial.write_all(&tx).unwrap();
        serial.flush().unwrap();
        n = 0;
        let deadline2 = Instant::now() + Duration::from_millis(500);
        while n < 8 && Instant::now() < deadline2 {
            match serial.read(&mut buf[n..]) {
                Ok(k) if k > 0 => { n += k; println!("  post-reset rx {} bytes: {:02x?}", n, &buf[..n]); }
                _ => {}
            }
        }
        if n == 0 {
            println!("  NO RESPONSE after DTR/RTS reset");
        }
        println!("=== Diagnostic done ===");
    }

    /// Non-destructive: test handshake, baud switch, MID read, unprotect.
    /// Run with: cargo test bk_live_handshake -- --ignored --nocapture
    #[test]
    #[ignore = "requires Air8101 physically connected on COM6"]
    fn bk_live_handshake() {
        let port = "COM6";
        println!("\n=== BK7231 live handshake test on {port} ===");

        let mut serial = serialport::new(port, 115_200)
            .timeout(Duration::from_millis(200))
            .open()
            .expect("open COM6");

        // Enter bootloader
        println!("[1] get_bus …");
        let ok = get_bus(&mut *serial).expect("get_bus");
        assert!(ok, "Failed to enter bootloader — is Air8101 connected?");
        println!("    LinkCheck ACK received ✓");

        // Switch to 2 Mbps
        println!("[2] set_baud_rate(2_000_000) …");
        let ok = set_baud_rate(&mut *serial, 2_000_000, 200).expect("set_baud_rate");
        assert!(ok, "Baud rate switch ACK failed");
        println!("    Baud rate switched ✓");

        // Get Flash MID
        println!("[3] get_flash_mid …");
        let mid = get_flash_mid(&mut *serial).expect("get_flash_mid");
        println!("    MID = 0x{:06x} ✓", mid);
        assert_ne!(mid, 0, "Flash MID should not be 0");

        // Unprotect (non-destructive SR write)
        println!("[4] unprotect_flash …");
        unprotect_flash(&mut *serial, mid).expect("unprotect_flash");
        println!("    Unprotected ✓");

        println!("\n=== Handshake PASSED ===");
    }

    /// Full closed-loop test: flash LuatOS firmware via air602_flash.exe subprocess,
    /// then immediately capture the device boot log and verify expected output.
    ///
    /// Run with: cargo test bk_live_full_flash -- --ignored --nocapture
    #[test]
    #[ignore = "DESTRUCTIVE — requires Air8101 on COM6 and D:\\bk7258\\LuatOS-SoC_V2013_Air8101.soc"]
    fn bk_live_full_flash() {
        let soc = r"D:\bk7258\LuatOS-SoC_V2013_Air8101.soc";
        let port_name = "COM6";

        println!("\n=== BK7258 full flash+boot test (subprocess) on {port_name} ===");

        // Extract .soc and parse info.json
        let info = read_soc_info(soc).expect("read_soc_info");
        println!("SOC chip: {}", info.chip.chip_type);
        println!("ROM file: {}", info.rom.file);

        let tempdir = tempfile::tempdir().expect("tempdir");
        {
            let file = std::fs::File::open(soc).expect("open soc");
            let mut archive = zip::ZipArchive::new(file).expect("zip");
            archive.extract(tempdir.path()).expect("extract");
        }

        let exe_path = tempdir.path().join("air602_flash.exe");
        assert!(exe_path.exists(), "air602_flash.exe not found in .soc — is this a BK7258 firmware?");

        let rom_path = tempdir.path().join(&info.rom.file);
        let log_br: u32 = info.user.as_ref()
            .and_then(|u| u.log_br.as_deref())
            .and_then(|s| s.parse().ok())
            .unwrap_or(2_000_000);

        println!("[1] Flashing via air602_flash.exe…");
        println!("    exe: {}", exe_path.display());
        println!("    rom: {}", rom_path.display());
        println!("    port: {port_name}, log_br: {log_br}");

        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        // flash_via_subprocess prints progress via AppHandle events — in test context we use a
        // dummy handle created via a tauri::test builder.  However, since we only need the
        // function's side-effects (subprocess + serial read) and its return value, we call the
        // helper directly after building a throwaway Tauri test app.
        //
        // Simpler: call the function logic inline so we can print to stdout.
        let port_num: String = port_name.chars().filter(|c| c.is_ascii_digit()).collect();

        println!("[2] Launching subprocess (this takes ~37 s)…");
        let start = Instant::now();
        let mut child = std::process::Command::new(&exe_path)
            .args(["download", "-p", &port_num, "-b", "2000000", "-s", "0", "-i"])
            .arg(&rom_path)
            .current_dir(tempdir.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn air602_flash.exe");

        // Drain stdout in background
        let stdout = child.stdout.take().expect("stdout");
        let reader_thread = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let mut lines = Vec::new();
            for line in BufReader::new(stdout).lines().filter_map(|l| l.ok()) {
                let t = line.trim().to_string();
                if !t.is_empty() {
                    println!("    [exe] {t}");
                    lines.push(t);
                }
            }
            lines
        });

        let _status = child.wait().expect("wait");
        let exe_lines = reader_thread.join().expect("reader thread");
        println!("    Subprocess exited after {:.1}s", start.elapsed().as_secs_f32());

        let flash_ok = exe_lines.iter().any(|l| l.contains("All Finished"));
        if !flash_ok {
            println!("    [WARN] 'All Finished' not found in exe output — continuing anyway");
        }

        println!("[3] Opening {port_name} @ {log_br} for boot log capture…");
        let mut log_port = serialport::new(port_name, log_br)
            .timeout(Duration::from_millis(200))
            .open()
            .expect("open log port");

        println!("[4] Reading boot log for {}s…", LOG_CAPTURE_SECS);
        let mut log_bytes: Vec<u8> = Vec::new();
        let mut buf = [0u8; 512];
        let deadline = Instant::now() + Duration::from_secs(LOG_CAPTURE_SECS);
        while Instant::now() < deadline {
            if cancel.load(Ordering::Relaxed) { break; }
            match log_port.read(&mut buf) {
                Ok(n) if n > 0 => { log_bytes.extend_from_slice(&buf[..n]); }
                _ => {}
            }
        }

        println!("    Boot log: {} bytes", log_bytes.len());
        let log_text = String::from_utf8_lossy(&log_bytes);
        let preview: String = log_text.chars().take(800).collect();
        println!("    Preview:\n---\n{preview}\n---");

        let boot_kw = ["luat:", "ap0:", "ap1:", "LuatOS", "EasyFlash"];
        let passed = boot_kw.iter().any(|kw| log_text.contains(kw));
        assert!(
            passed,
            "No expected boot keywords found in {} bytes.\nExpected one of: {:?}\nLog:\n{}",
            log_bytes.len(), boot_kw, preview
        );

        println!("\n=== Full Flash Test PASSED — firmware booted and produced expected output ===");
    }
}
