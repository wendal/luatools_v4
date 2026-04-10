// flash/ec618.rs
// Stub implementation of the EC618 / EC718 (Air780E / Air780EP) ISP flasher.
//
// The EC618 / EC718 series (made by Eigencomm) use a UART-based FDL
// (Firmware Download) protocol for in-system programming.  This module will
// grow into a full implementation once the protocol details are finalised.
//
// Architectural reference: yuzhan-tech/luatos-tools (src/flash/)

use anyhow::{bail, Result};

use super::{Flasher, FlashProgress, ProgressCallback};

/// Configuration for the EC618 / EC718 flasher.
#[derive(Debug, Clone)]
pub struct Ec618Config {
    /// Serial port path, e.g. `/dev/ttyUSB0` or `COM3`.
    pub port: String,
    /// Baud rate used during the download phase (typically 921600).
    pub baud_rate: u32,
    /// Timeout in milliseconds for individual serial operations.
    pub timeout_ms: u64,
}

impl Default for Ec618Config {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud_rate: 921_600,
            timeout_ms: 3_000,
        }
    }
}

/// ISP flasher for EC618 / EC718 chips (Air780E / Air780EP family).
pub struct Ec618Flasher {
    config: Ec618Config,
}

impl Ec618Flasher {
    pub fn new(config: Ec618Config) -> Self {
        Self { config }
    }
}

impl Flasher for Ec618Flasher {
    fn connect(&mut self) -> Result<()> {
        // TODO: open serial port, send FDL handshake, negotiate baud rate.
        log::info!(
            "[EC618] connect() — port={} baud={}",
            self.config.port,
            self.config.baud_rate
        );
        bail!("EC618 flasher: connect() not yet implemented")
    }

    fn erase(&mut self) -> Result<()> {
        // TODO: send FDL erase command with target address and length.
        log::info!("[EC618] erase()");
        bail!("EC618 flasher: erase() not yet implemented")
    }

    fn write(&mut self, _data: &[u8], on_progress: &ProgressCallback) -> Result<()> {
        // TODO: chunk data into FDL DATA packets, send and await ACKs.
        on_progress(FlashProgress {
            stage: "Writing".into(),
            written: 0,
            total: _data.len() as u64,
        });
        bail!("EC618 flasher: write() not yet implemented")
    }

    fn verify(&mut self, _data: &[u8]) -> Result<()> {
        // TODO: request checksum from chip and compare with local CRC.
        log::info!("[EC618] verify()");
        bail!("EC618 flasher: verify() not yet implemented")
    }

    fn reset(&mut self) -> Result<()> {
        // TODO: send FDL RESET command or toggle DTR/RTS to restart the chip.
        log::info!("[EC618] reset()");
        bail!("EC618 flasher: reset() not yet implemented")
    }
}
