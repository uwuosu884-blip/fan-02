# omen15-fan

**New here / not a Linux expert?** Skip straight to
[INSTALL_SIMPLE.md](INSTALL_SIMPLE.md) — plain-language, copy-paste steps.

Custom, from-scratch fan control for the **HP Omen 15-en1003ur** (DMI board
**88D2**) on Arch Linux, written in Rust. Talks directly to the embedded
controller (EC) via the kernel's `ec_sys` debugfs interface — no vendor
software, no unsupported third-party kernel module required.

> **Why build this instead of using an existing tool?** Several projects
> (pankha, omencore, omen-fan) have recently added entries for board 88D2,
> but EC register layouts can differ across firmware revisions of the same
> board id. This tool ships the commonly-reported 88D2 register map as a
> starting point, but leads with a `probe`/calibration workflow so you
> verify it against your exact machine before trusting it — see
> [docs/CALIBRATION.md](docs/CALIBRATION.md). **Do that first.**

## What it does

- Reads CPU temperature from `hwmon` (`k10temp`, correct for your Ryzen CPU).
- Reads/writes EC registers for fan control/target/read via
  `/sys/kernel/debug/ec/ec0/io` (the `ec_sys` kernel module, loaded with
  `write_support=1`).
- A configurable temperature → fan-speed curve with hysteresis, run as a
  systemd service.
- Manual override (`set --percent`/`--rpm`), and a one-command way to hand
  control back to the BIOS (`auto`).
- Hard RPM ceiling, board-id safety check (bypassable with `--force` only
  once you've verified things yourself), and automatic revert-to-auto on
  exit/crash/Ctrl+C.
- A `probe --watch` mode for safely discovering/confirming the exact
  register map on your firmware before you trust any writes.

## Build

Toolchain lives outside your workspace budget, same convention as always:

```bash
if [ ! -x /opt/rust/cargo/bin/cargo ]; then
  sudo mkdir -p /opt/rust && sudo chown "$USER" /opt/rust
  export RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
fi
. ~/.profile  # exports RUSTUP_HOME/CARGO_HOME/PATH, see that file
cd omen15-fan
cargo build --release
```

## Install — option A: plain binary + systemd

```bash
sudo install -m755 target/release/omen15-fan /usr/local/bin/omen15-fan
sudo omen15-fan init-config                 # writes /etc/omen15-fan/config.toml
sudo omen15-fan service install             # systemd unit + ec_sys modprobe config
sudoedit /etc/omen15-fan/config.toml        # review registers/curve — see CALIBRATION.md
sudo omen15-fan probe --watch               # CONFIRM the register map first!
sudo systemctl enable --now omen15-fan.service
journalctl -u omen15-fan -f                 # watch it behave for a while
```

## Install — option B: Arch package

```bash
cd arch
makepkg -si
sudo omen15-fan probe --watch   # confirm registers before enabling the service
sudo systemctl enable --now omen15-fan.service
```

## CLI reference

```
omen15-fan status                 # board id, temps, current register values
omen15-fan calibrate              # guided wizard: heats CPU, shows what changed, offers a live test
omen15-fan probe                  # one-shot hex dump of all 256 EC bytes
omen15-fan probe --watch          # live diff — manual calibration tool, see docs/CALIBRATION.md
omen15-fan set --percent 60       # manual: 60% of max_rpm
omen15-fan set --rpm 3200         # manual: fixed RPM, clamped to max_rpm
omen15-fan auto                   # hand control back to the BIOS
omen15-fan curve                  # run the configured temp curve in the foreground
omen15-fan init-config            # write the default config if missing
omen15-fan service install        # install systemd unit + ec_sys module config + suspend/resume hook
omen15-fan service uninstall      # remove them (config left in place)
omen15-fan resume-fix             # manual one-off fix if things stop responding after lid close/open
```

All commands accept `--config <path>` (default `/etc/omen15-fan/config.toml`)
and `--force` to bypass the board-id check.

## If `ec_sys` won't load with write support

Some distro kernels build `ec_sys` without `CONFIG_ACPI_EC_DEBUGFS` write
support, or disallow runtime EC writes for "safety." If `modprobe ec_sys
write_support=1` loads but `/sys/kernel/debug/ec/ec0/io` stays read-only:

- Check `dmesg | grep -i ec_sys` for refusal messages.
- As a fallback, the `acpi_call` kernel module (`pacman -S acpi_call-dkms`)
  lets you invoke the EC through ACPI methods instead of raw debugfs; that
  would need a small follow-up patch to `src/ec.rs` to add an alternate
  backend. Tell me if `ec_sys` doesn't work for you and I'll add it.

## Safety

- `max_rpm` in the config is a hard ceiling enforced in code, not just a
  curve suggestion.
- Exiting the `curve` command (Ctrl+C, `systemctl stop`, or a crash caught by
  the signal handler) writes `control=0`, handing the fan back to the BIOS.
- The board-id check refuses to run `set`/`curve` on hardware that doesn't
  report board `88D2`, unless you pass `--force`.
- None of this has been tested against a real HP EC from this environment —
  it was written and unit-tested (curve math) in a sandbox with no such
  hardware. Follow docs/CALIBRATION.md before trusting it unattended.
