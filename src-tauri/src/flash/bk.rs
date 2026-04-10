// flash/bk.rs
// Stub implementation of the Beken (BK) chip ISP flasher.
//
// Beken chips (e.g. BK7231N / BK7231T used in some LuatOS modules) use their
// own UART-based ISP protocol.  This module will grow into a full
// implementation once the protocol details are finalised.

use anyhow::{bail, Result};

use super::{Flasher, FlashProgress, ProgressCallback};

/// Configuration for the Beken ISP flasher.
#[derive(Debug, Clone)]
pub struct BkConfig {
    /// Serial port path, e.g. `/dev/ttyUSB0` or `COM3`.
    pub port: String,
    /// Baud rate used during the download phase (typically 921600).
    pub baud_rate: u32,
    /// Timeout in milliseconds for individual serial operations.
    pub timeout_ms: u64,
}

impl Default for BkConfig {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud_rate: 921_600,
            timeout_ms: 3_000,
        }
    }
}

/// ISP flasher for Beken (BK) chips.
pub struct BkFlasher {
    config: BkConfig,
}

impl BkFlasher {
    pub fn new(config: BkConfig) -> Self {
        Self { config }
    }
}

impl Flasher for BkFlasher {
    fn connect(&mut self) -> Result<()> {
        // TODO: open serial port, send Beken ISP sync byte, await ACK.
        log::info!(
            "[BK] connect() — port={} baud={}",
            self.config.port,
            self.config.baud_rate
        );
        bail!("BK flasher: connect() not yet implemented")
    }

    fn erase(&mut self) -> Result<()> {
        // TODO: send BK erase command.
        log::info!("[BK] erase()");
        bail!("BK flasher: erase() not yet implemented")
    }

    fn write(&mut self, _data: &[u8], on_progress: &ProgressCallback) -> Result<()> {
        // TODO: chunk data into BK ISP write packets.
        on_progress(FlashProgress {
            stage: "Writing".into(),
            written: 0,
            total: _data.len() as u64,
        });
        bail!("BK flasher: write() not yet implemented")
    }

    fn verify(&mut self, _data: &[u8]) -> Result<()> {
        // TODO: request checksum and compare.
        log::info!("[BK] verify()");
        bail!("BK flasher: verify() not yet implemented")
    }

    fn reset(&mut self) -> Result<()> {
        // TODO: toggle DTR/RTS or send BK reset command.
        log::info!("[BK] reset()");
        bail!("BK flasher: reset() not yet implemented")
    }
}
