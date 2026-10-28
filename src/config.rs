use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/omen15-fan/config.toml";

/// EC register offsets for the fan control scheme. Defaults below come from
/// the register map that other projects (e.g. `pankha`) have confirmed on
/// board 88D2 (HP Omen 15-en1xxx). Different BIOS/EC firmware revisions of
/// the *same* board id have been observed to use different offsets, so
/// before relying on `curve`/`set`, run `omen15-fan probe --watch` and
/// confirm these against your own machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterMap {
    /// 0 = BIOS controls the fan automatically. Non-zero = manual control
    /// using `fan1_target` (and `fan2_target` if set) below.
    pub control: u8,
    /// Register the BIOS reads to know the fan's commanded speed, in units
    /// of 100 RPM (0x1E == 3000 RPM).
    pub fan1_target: u8,
    /// Register that reports the fan's actual current speed (same units).
    pub fan1_read: u8,
    /// Second fan (GPU fan), if your machine exposes one independently.
    /// Leave as `None` until confirmed via `probe --watch`; until then both
    /// fans are driven together through fan1_target, matching what pankha
    /// documents for 88D2.
    pub fan2_target: Option<u8>,
    pub fan2_read: Option<u8>,
}

impl Default for RegisterMap {
    fn default() -> Self {
        RegisterMap {
            control: 0x15,
            fan1_target: 0x19,
            fan1_read: 0x11,
            fan2_target: None,
            fan2_read: None,
        }
    }
}

/// One point of the temperature -> fan speed curve. `speed_percent` is a
/// percentage of `max_rpm`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CurvePoint {
    pub temp_c: f32,
    pub speed_percent: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub ec_path: String,
    pub registers: RegisterMap,
    /// Hard ceiling in RPM. omen15-fan will never command a speed above
    /// this value, regardless of the curve or manual `set` command.
    pub max_rpm: u32,
    /// Minimum RPM floor used while in curve/auto mode (idle speed).
    pub idle_rpm: u32,
    pub poll_interval_ms: u64,
    /// Avoid oscillating the fan for +/- this many degrees around a curve
    /// breakpoint.
    pub hysteresis_c: f32,
    pub curve: Vec<CurvePoint>,
    /// hwmon chip name to source CPU temperature from. `k10temp` is correct
    /// for your Ryzen CPU; `auto` tries to find it automatically.
    pub hwmon_chip: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ec_path: crate::ec::DEFAULT_EC_PATH.to_string(),
            registers: RegisterMap::default(),
            max_rpm: 5500,
            idle_rpm: 2000,
            poll_interval_ms: 2000,
            hysteresis_c: 3.0,
            hwmon_chip: "auto".to_string(),
            curve: vec![
                CurvePoint { temp_c: 40.0, speed_percent: 0 },   // idle_rpm applies below this
                CurvePoint { temp_c: 55.0, speed_percent: 35 },
                CurvePoint { temp_c: 65.0, speed_percent: 55 },
                CurvePoint { temp_c: 75.0, speed_percent: 75 },
                CurvePoint { temp_c: 85.0, speed_percent: 90 },
                CurvePoint { temp_c: 92.0, speed_percent: 100 },
            ],
        }
    }
}

impl Config {
    pub fn load_or_default(path: &str) -> Result<Config> {
        if Path::new(path).exists() {
            let text = fs::read_to_string(path)
                .with_context(|| format!("reading config {path}"))?;
            let cfg: Config =
                toml::from_str(&text).with_context(|| format!("parsing config {path}"))?;
            cfg.validate()?;
            Ok(cfg)
        } else {
            Ok(Config::default())
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.max_rpm == 0 || self.max_rpm > 8000 {
            anyhow::bail!("max_rpm {} looks wrong (expected roughly 2000-6500)", self.max_rpm);
        }
        if self.idle_rpm > self.max_rpm {
            anyhow::bail!("idle_rpm cannot be greater than max_rpm");
        }
        let mut last_temp = f32::MIN;
        for p in &self.curve {
            if p.temp_c < last_temp {
                anyhow::bail!("curve temp_c points must be sorted ascending");
            }
            if p.speed_percent > 100 {
                anyhow::bail!("curve speed_percent must be 0-100");
            }
            last_temp = p.temp_c;
        }
        Ok(())
    }

    pub fn save(&self, path: &str) -> Result<()> {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text).with_context(|| format!("writing config {path}"))
    }

    /// Convert a target RPM into the raw register value (units of 100 RPM),
    /// clamped to [0, max_rpm].
    pub fn rpm_to_raw(&self, rpm: u32) -> u8 {
        let clamped = rpm.min(self.max_rpm);
        (clamped / 100).min(255) as u8
    }

    pub fn raw_to_rpm(&self, raw: u8) -> u32 {
        raw as u32 * 100
    }
}
