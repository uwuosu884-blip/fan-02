use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Finds the best CPU temperature source under /sys/class/hwmon.
/// `chip` may be "auto" or an exact hwmon chip name (e.g. "k10temp").
pub struct TempSensor {
    inputs: Vec<PathBuf>,
    chip_found: String,
}

impl TempSensor {
    pub fn discover(chip: &str) -> Result<TempSensor> {
        let base = PathBuf::from("/sys/class/hwmon");
        let entries = fs::read_dir(&base).context("reading /sys/class/hwmon")?;

        // Preference order when chip == "auto": k10temp (AMD Ryzen, your
        // CPU) first, then common fallbacks.
        let preferred = ["k10temp", "coretemp", "zenpower", "acpitz"];

        let mut candidates: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let name_path = entry.path().join("name");
            if let Ok(name) = fs::read_to_string(&name_path) {
                candidates.push((name.trim().to_string(), entry.path()));
            }
        }

        let chosen = if chip != "auto" {
            candidates
                .iter()
                .find(|(n, _)| n == chip)
                .cloned()
        } else {
            preferred
                .iter()
                .find_map(|want| candidates.iter().find(|(n, _)| n == want).cloned())
                .or_else(|| candidates.first().cloned())
        };

        let (chip_found, dir) = chosen.with_context(|| {
            format!(
                "no hwmon sensor found (looked for '{chip}'); available: {:?}",
                candidates.iter().map(|c| &c.0).collect::<Vec<_>>()
            )
        })?;

        let mut inputs = Vec::new();
        for entry in fs::read_dir(&dir)?.flatten() {
            let fname = entry.file_name();
            let fname = fname.to_string_lossy();
            if fname.starts_with("temp") && fname.ends_with("_input") {
                inputs.push(entry.path());
            }
        }
        if inputs.is_empty() {
            anyhow::bail!("hwmon chip '{chip_found}' has no temp*_input files");
        }
        inputs.sort();

        Ok(TempSensor { inputs, chip_found })
    }

    pub fn chip_name(&self) -> &str {
        &self.chip_found
    }

    /// Returns the hottest reading among this chip's temp inputs, in
    /// degrees Celsius. CPU packages often expose several (Tdie, Tctl,
    /// per-core) — the max is the safe choice for fan-curve purposes.
    pub fn read_max_c(&self) -> Result<f32> {
        let mut max_milli: i64 = i64::MIN;
        for p in &self.inputs {
            if let Ok(text) = fs::read_to_string(p) {
                if let Ok(v) = text.trim().parse::<i64>() {
                    max_milli = max_milli.max(v);
                }
            }
        }
        if max_milli == i64::MIN {
            anyhow::bail!("could not read any temp*_input under {}", self.chip_found);
        }
        Ok(max_milli as f32 / 1000.0)
    }
}
