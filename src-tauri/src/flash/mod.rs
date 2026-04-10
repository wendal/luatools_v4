// flash/mod.rs
// Generic flasher trait used by all chip-specific ISP implementations.
#![allow(dead_code)]

pub mod bk;
pub mod ec618;

use anyhow::Result;

/// Progress information reported during a flash operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FlashProgress {
    /// Human-readable stage name (e.g. "Connecting", "Erasing", "Writing").
    pub stage: String,
    /// Bytes written so far.
    pub written: u64,
    /// Total bytes to write (0 if unknown).
    pub total: u64,
}

/// Callback type invoked periodically to report flash progress.
pub type ProgressCallback = Box<dyn Fn(FlashProgress) + Send + Sync>;

/// Common interface implemented by every chip-specific flasher.
///
/// Each implementation is responsible for the full ISP lifecycle:
/// open serial port → handshake/connect → erase → write → verify → reset.
pub trait Flasher {
    /// Connect to the target chip and enter ISP/download mode.
    fn connect(&mut self) -> Result<()>;

    /// Erase the flash region that will receive the new firmware.
    fn erase(&mut self) -> Result<()>;

    /// Write `data` to the chip, reporting progress via `on_progress`.
    fn write(&mut self, data: &[u8], on_progress: &ProgressCallback) -> Result<()>;

    /// Verify that flash contents match `data`.
    fn verify(&mut self, data: &[u8]) -> Result<()>;

    /// Reset the chip and exit ISP mode so it boots the new firmware.
    fn reset(&mut self) -> Result<()>;
}
