use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// Standard debugfs path exposed by the in-kernel `ec_sys` module.
pub const DEFAULT_EC_PATH: &str = "/sys/kernel/debug/ec/ec0/io";
pub const EC_SIZE: usize = 256;

pub struct Ec {
    path: PathBuf,
}

impl Ec {
    pub fn open(path: Option<&str>) -> Self {
        Ec {
            path: PathBuf::from(path.unwrap_or(DEFAULT_EC_PATH)),
        }
    }

    pub fn check_available(&self) -> Result<()> {
        if !self.path.exists() {
            anyhow::bail!(
                "{} does not exist.\n\
                 Load the ec_sys module with write support:\n\
                 \n    sudo modprobe ec_sys write_support=1\n\n\
                 (omen15-fan service install sets this up permanently via\n\
                 /etc/modprobe.d and /etc/modules-load.d.)",
                self.path.display()
            );
        }
        Ok(())
    }

    fn open_file(&self, write: bool) -> Result<File> {
        self.check_available()?;
        OpenOptions::new()
            .read(true)
            .write(write)
            .open(&self.path)
            .with_context(|| format!("opening {}", self.path.display()))
    }

    /// Read a single byte at `offset`.
    pub fn read_byte(&self, offset: u8) -> Result<u8> {
        let mut f = self.open_file(false)?;
        f.seek(SeekFrom::Start(offset as u64))?;
        let mut buf = [0u8; 1];
        f.read_exact(&mut buf)
            .with_context(|| format!("reading EC offset 0x{offset:02X}"))?;
        Ok(buf[0])
    }

    /// Read the full 256-byte EC address space (used by `probe`).
    pub fn read_all(&self) -> Result<[u8; EC_SIZE]> {
        let mut f = self.open_file(false)?;
        let mut buf = [0u8; EC_SIZE];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf).context("reading full EC range")?;
        Ok(buf)
    }

    /// Write a single byte at `offset`. Callers are responsible for any
    /// safety clamping — this module does no interpretation of values.
    pub fn write_byte(&self, offset: u8, value: u8) -> Result<()> {
        let mut f = self.open_file(true)?;
        f.seek(SeekFrom::Start(offset as u64))?;
        f.write_all(&[value])
            .with_context(|| format!("writing 0x{value:02X} to EC offset 0x{offset:02X}"))?;
        Ok(())
    }
}
