// flash/mod.rs
#![allow(dead_code)]

pub mod bk;
pub mod ec618;

use anyhow::Result;

#[derive(Debug, Clone, serde::Serialize)]
pub struct FlashProgress {
    pub stage: String,
    pub written: u64,
    pub total: u64,
}

pub type ProgressCallback = Box<dyn Fn(FlashProgress) + Send + Sync>;

pub trait Flasher {
    fn connect(&mut self) -> Result<()>;
    fn erase(&mut self) -> Result<()>;
    fn write(&mut self, data: &[u8], on_progress: &ProgressCallback) -> Result<()>;
    fn verify(&mut self, data: &[u8]) -> Result<()>;
    fn reset(&mut self) -> Result<()>;
}
