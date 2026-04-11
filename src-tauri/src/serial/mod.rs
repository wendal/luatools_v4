// serial/mod.rs
#![allow(dead_code)]

pub mod log;

use serde::{Deserialize, Serialize};

// Re-export log macros via absolute path to avoid shadowing by the `log` submodule.
use ::log::{debug, info, warn};

/// Metadata about a serial port visible on the host system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortInfo {
    /// System port name (e.g. `COM3` on Windows, `/dev/ttyUSB0` on Linux).
    pub port_name: String,
    /// Optional USB vendor ID (hex string, e.g. `"1a86"`).
    pub vid: Option<String>,
    /// Optional USB product ID (hex string, e.g. `"7523"`).
    pub pid: Option<String>,
    /// Optional manufacturer string reported by the USB device.
    pub manufacturer: Option<String>,
    /// Optional product / friendly name.
    pub product: Option<String>,
    /// Optional USB serial number string.
    pub serial_number: Option<String>,
}

/// Return a list of all serial ports currently visible on the host system.
///
/// Uses the `serialport` crate internally.  On failure returns an empty list
/// rather than propagating an error, so the frontend always gets a valid JSON
/// array.
pub fn list_ports() -> Vec<PortInfo> {
    debug!("[serial] Enumerating serial ports via serialport crate …");
    let raw = serialport::available_ports().unwrap_or_else(|e| {
        warn!("[serial] serialport::available_ports() failed: {}", e);
        Vec::new()
    });
    info!("[serial] Raw port count from OS: {}", raw.len());

    let result: Vec<PortInfo> = raw.into_iter().map(|p| {
        let (vid, pid, manufacturer, product, serial_number) =
            if let serialport::SerialPortType::UsbPort(ref info) = p.port_type {
                (
                    Some(format!("{:04x}", info.vid)),
                    Some(format!("{:04x}", info.pid)),
                    info.manufacturer.clone(),
                    info.product.clone(),
                    info.serial_number.clone(),
                )
            } else {
                (None, None, None, None, None)
            };
        info!(
            "[serial]   port={} vid={:?} pid={:?} product={:?}",
            p.port_name, vid, pid, product,
        );
        PortInfo { port_name: p.port_name, vid, pid, manufacturer, product, serial_number }
    }).collect();

    info!("[serial] Returning {} port(s) to frontend", result.len());
    result
}

/// A simple ring-buffer wrapper for accumulating bytes read from a serial
/// port before they are parsed by the ISP protocol layer.
///
/// The buffer capacity defaults to 4 KiB but can be configured.
pub struct SerialBuffer {
    inner: Vec<u8>,
    capacity: usize,
}

impl SerialBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Push raw bytes into the buffer, discarding the oldest bytes when the
    /// capacity limit is reached.
    pub fn push(&mut self, data: &[u8]) {
        if self.inner.len() + data.len() > self.capacity {
            let overflow = (self.inner.len() + data.len()) - self.capacity;
            self.inner.drain(..overflow);
        }
        self.inner.extend_from_slice(data);
    }

    /// Drain all accumulated bytes, leaving the buffer empty.
    pub fn drain(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.inner)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
