use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/omen15-fan/config.toml";

/// How a "read" register's raw byte should be interpreted for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadEncoding {
    /// raw * 100 = RPM (confirmed convention for the *target* registers;
    /// some boards' read-back registers mirror this too).
    Rpm100,
    /// Period-encoded tachometer (smaller value = faster fan). We don't yet
    /// have a calibrated raw->RPM formula for this on your exact firmware,
    /// so it's shown as a raw number, not a guessed RPM.
    PeriodRaw,
}

/// EC register offsets for the fan control scheme.
///
/// The defaults below are the map **confirmed by live testing** on an HP
/// Omen 15-en1003ur (board 88D2): `0x62`/`0x63`/`0x34`/`0x35` — this is the
/// older, widely-used HP Omen EC layout (the same one the long-standing
/// `omen-fan`/`hp-omen-linux-module` projects use), not the newer scheme
/// `pankha` documents for 88D2 (`0x15`/`0x19`), which tested as inert on
/// this unit. See docs/CALIBRATION.md for how this was found, and keep an
/// eye out for other 88D2 firmware revisions needing yet another map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterMap {
    /// Manual-control switch register. Write `control_enable_value` to turn
    /// manual mode on, `0` to hand back to the BIOS.
    pub control: u8,
    /// The value that actually means "manual" for this firmware. Confirmed
    /// as `6` on en1003ur (NOT `1` — that's what the newer pankha-style
    /// scheme uses on other 88D2 firmware).
    pub control_enable_value: u8,
    /// Optional EC watchdog/countdown register. Some firmware automatically
    /// hands control back to the BIOS once this counts down, unless the
    /// host keeps resetting it to 0. omen15-fan does this automatically
    /// every poll while a command is active. This is actually a *safety
    /// feature*: if this tool crashes without cleaning up, the firmware
    /// itself will revert to auto shortly after.
    pub countdown: Option<u8>,
    /// Write target for fan 1 (CPU), units of 100 RPM (0x1E == 3000 RPM).
    pub fan1_target: u8,
    /// Best-effort read-back register for fan 1's actual speed.
    pub fan1_read: u8,
    pub fan1_read_encoding: ReadEncoding,
    /// Fan 2 (GPU), if your machine exposes/needs an independent register.
    pub fan2_target: Option<u8>,
    pub fan2_read: Option<u8>,
    pub fan2_read_encoding: ReadEncoding,
}

impl Default for RegisterMap {
    fn default() -> Self {
        RegisterMap {
            control: 0x62,
            control_enable_value: 6,
            countdown: Some(0x63),
            fan1_target: 0x34,
            fan1_read: 0x2E,
            fan1_read_encoding: ReadEncoding::PeriodRaw,
            fan2_target: Some(0x35),
            fan2_read: Some(0x2F),
            fan2_read_encoding: ReadEncoding::PeriodRaw,
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
    /// Speed used ONLY when the curve resolves to 0% (temperature below
    /// your first curve point). This is NOT a floor applied to other curve
    /// percentages — e.g. a "10%" point is followed exactly, even if the
    /// resulting RPM is below this value. Set to 0 to let the fan fully
    /// stop below that temperature.
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
        if self.registers.control_enable_value == 0 {
            anyhow::bail!("registers.control_enable_value must be non-zero (0 means auto/off)");
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

    /// Human-readable rendering of a read-register's raw byte, respecting
    /// its configured encoding.
    pub fn format_read(&self, raw: u8, encoding: ReadEncoding) -> String {
        match encoding {
            ReadEncoding::Rpm100 => format!("~{} RPM", self.raw_to_rpm(raw)),
            ReadEncoding::PeriodRaw => {
                format!("raw period {raw} (lower = faster; not yet calibrated to RPM)")
            }
        }
    }
}
