use anyhow::Result;
use std::fs;

/// Board id HP ships for the Omen 15-en1xxx family (your en1003ur included).
pub const EXPECTED_BOARD_ID: &str = "88D2";

#[derive(Debug, Clone)]
pub struct DmiInfo {
    pub board_name: String,
    pub product_name: String,
    pub sys_vendor: String,
}

fn read_trim(path: &str) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

impl DmiInfo {
    pub fn read() -> Result<Self> {
        let board_name = read_trim("/sys/class/dmi/id/board_name");
        let product_name = read_trim("/sys/class/dmi/id/product_name");
        let sys_vendor = read_trim("/sys/class/dmi/id/sys_vendor");
        if board_name.is_empty() {
            anyhow::bail!(
                "could not read /sys/class/dmi/id/board_name (need root, or DMI not exposed)"
            );
        }
        Ok(Self {
            board_name,
            product_name,
            sys_vendor,
        })
    }

    pub fn matches_expected(&self) -> bool {
        self.board_name.eq_ignore_ascii_case(EXPECTED_BOARD_ID)
    }

    pub fn require_known_board(&self, force: bool) -> Result<()> {
        if self.matches_expected() || force {
            Ok(())
        } else {
            anyhow::bail!(
                "detected board '{}' (product '{}'), expected '{}'.\n\
                 This tool writes directly to embedded-controller registers that were only\n\
                 verified against board {}. Writing to the wrong board can make the fan do\n\
                 something unexpected. Re-run with --force if you have confirmed (via `probe`)\n\
                 that the same registers apply to your board.",
                self.board_name,
                self.product_name,
                EXPECTED_BOARD_ID,
                EXPECTED_BOARD_ID
            )
        }
    }
}

pub fn ensure_root() -> Result<()> {
    let uid = unsafe { libc::geteuid() };
    if uid != 0 {
        anyhow::bail!("this command needs root (EC and DMI access) — re-run with sudo");
    }
    Ok(())
}
