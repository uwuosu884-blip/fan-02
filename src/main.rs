mod config;
mod curve;
mod dmi;
mod ec;
mod hwmon;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use dmi::DmiInfo;
use ec::Ec;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "omen15-fan",
    version,
    about = "Direct EC fan control for HP Omen 15-en1xxx (board 88D2) on Linux.\n\
             DISCLAIMER: writes directly to embedded-controller registers. \
             Verify the register map with `probe --watch` before trusting `set`/`curve`."
)]
struct Cli {
    #[arg(long, global = true, default_value = config::DEFAULT_CONFIG_PATH)]
    config: String,

    /// Skip the board-id (DMI) safety check. Only use once you've confirmed
    /// via `probe` that these registers behave correctly on your machine.
    #[arg(long, global = true)]
    force: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show DMI board info, hwmon temperature and current EC register values.
    Status,
    /// Dump the full 256-byte EC address space, or watch it live for changes
    /// (use this to confirm/discover the register map on your exact
    /// firmware — see docs/CALIBRATION.md).
    Probe {
        /// Keep polling and only print bytes that changed since last poll.
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value_t = 300)]
        interval_ms: u64,
    },
    /// Switch to manual control and set a fixed fan speed.
    Set {
        /// Target speed as a percentage of max_rpm (0-100).
        #[arg(long, conflicts_with = "rpm")]
        percent: Option<u8>,
        /// Target speed in RPM, clamped to max_rpm from the config.
        #[arg(long)]
        rpm: Option<u32>,
    },
    /// Hand fan control back to the BIOS.
    Auto,
    /// Run the temperature-driven fan curve in the foreground (this is what
    /// the systemd service invokes).
    Curve,
    /// Guided calibration wizard: heats the CPU briefly, watches which EC
    /// bytes change, and (optionally, with your confirmation) runs a short
    /// safe manual-speed test so you can confirm the register map actually
    /// works on your machine. This is the easiest way to answer "will this
    /// work on my exact laptop?".
    Calibrate {
        /// How long to run the built-in CPU heater for, in seconds.
        #[arg(long, default_value_t = 20)]
        seconds: u64,
        /// Skip the heating step (only diff EC state at two points in time
        /// you control manually, e.g. before/after a game).
        #[arg(long)]
        no_heat: bool,
    },
    /// Write out the default config file (if one doesn't already exist).
    InitConfig,

    /// Install or remove the systemd service + ec_sys module configuration.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    Install,
    Uninstall,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Commands::Status => cmd_status(&cli),
        Commands::Probe { watch, interval_ms } => cmd_probe(&cli, *watch, *interval_ms),
        Commands::Set { percent, rpm } => cmd_set(&cli, *percent, *rpm),
        Commands::Auto => cmd_auto(&cli),
        Commands::Curve => cmd_curve(&cli),
        Commands::Calibrate { seconds, no_heat } => cmd_calibrate(&cli, *seconds, *no_heat),
        Commands::InitConfig => cmd_init_config(&cli),
        Commands::Service { action } => cmd_service(action),
    }
}

fn load_config(cli: &Cli) -> Result<Config> {
    Config::load_or_default(&cli.config)
}

fn cmd_status(cli: &Cli) -> Result<()> {
    let cfg = load_config(cli)?;
    let dmi = DmiInfo::read().unwrap_or_else(|_| dmi::DmiInfo {
        board_name: "<unreadable, try sudo>".into(),
        product_name: "".into(),
        sys_vendor: "".into(),
    });
    println!("== Board ==");
    println!(
        "  vendor: {}  product: {}  board: {}  (expected: {})",
        dmi.sys_vendor,
        dmi.product_name,
        dmi.board_name,
        dmi::EXPECTED_BOARD_ID
    );
    println!(
        "  board id match: {}",
        if dmi.matches_expected() { "yes" } else { "NO — see --force" }
    );

    println!("== Temperature ==");
    match hwmon::TempSensor::discover(&cfg.hwmon_chip) {
        Ok(sensor) => match sensor.read_max_c() {
            Ok(t) => println!("  {} max temp: {:.1} C", sensor.chip_name(), t),
            Err(e) => println!("  error reading temps: {e:#}"),
        },
        Err(e) => println!("  no hwmon sensor found: {e:#}"),
    }

    println!("== EC ({}) ==", cfg.ec_path);
    let ec = Ec::open(Some(&cfg.ec_path));
    match ec.check_available() {
        Ok(()) => {
            let r = &cfg.registers;
            let control = ec.read_byte(r.control)?;
            let target = ec.read_byte(r.fan1_target)?;
            let read1 = ec.read_byte(r.fan1_read)?;
            println!(
                "  control[0x{:02X}] = 0x{:02X} ({})",
                r.control,
                control,
                if control == 0 { "BIOS/auto" } else { "manual" }
            );
            println!(
                "  fan1_target[0x{:02X}] = 0x{:02X}  (~{} RPM commanded)",
                r.fan1_target,
                target,
                cfg.raw_to_rpm(target)
            );
            println!(
                "  fan1_read[0x{:02X}]   = 0x{:02X}  (~{} RPM reported)",
                r.fan1_read,
                read1,
                cfg.raw_to_rpm(read1)
            );
            if let (Some(t2), Some(r2)) = (r.fan2_target, r.fan2_read) {
                let target2 = ec.read_byte(t2)?;
                let read2 = ec.read_byte(r2)?;
                println!(
                    "  fan2_target[0x{:02X}] = 0x{:02X}  (~{} RPM commanded)",
                    t2,
                    target2,
                    cfg.raw_to_rpm(target2)
                );
                println!(
                    "  fan2_read[0x{:02X}]   = 0x{:02X}  (~{} RPM reported)",
                    r2,
                    read2,
                    cfg.raw_to_rpm(read2)
                );
            }
        }
        Err(e) => println!("  {e:#}"),
    }
    Ok(())
}

fn cmd_probe(cli: &Cli, watch: bool, interval_ms: u64) -> Result<()> {
    dmi::ensure_root()?;
    let cfg = load_config(cli)?;
    let ec = Ec::open(Some(&cfg.ec_path));
    ec.check_available()?;

    if !watch {
        let snapshot = ec.read_all()?;
        print_hex_grid(&snapshot);
        return Ok(());
    }

    println!(
        "Watching EC for changes every {interval_ms}ms. Press Ctrl+C to stop.\n\
         Tip: now press the OMEN key, change Performance mode in the BIOS/HP app,\n\
         or let the laptop heat up under load — the offsets that change are your\n\
         fan/thermal registers. Compare against README's known 88D2 map (0x11/0x15/0x19)."
    );
    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        ctrlc::set_handler(move || running.store(false, Ordering::SeqCst))
            .context("setting Ctrl+C handler")?;
    }

    let mut prev = ec.read_all()?;
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(interval_ms));
        let cur = ec.read_all()?;
        for i in 0..ec::EC_SIZE {
            if cur[i] != prev[i] {
                println!(
                    "offset 0x{:02X}: 0x{:02X} -> 0x{:02X}",
                    i, prev[i], cur[i]
                );
            }
        }
        prev = cur;
    }
    Ok(())
}

fn print_hex_grid(data: &[u8; ec::EC_SIZE]) {
    print!("     ");
    for col in 0..16 {
        print!(" {col:X} ");
    }
    println!();
    for row in 0..16 {
        print!("0x{:02X}:", row * 16);
        for col in 0..16 {
            print!(" {:02X}", data[row * 16 + col]);
        }
        println!();
    }
}

/// Interactive, mostly-automated calibration wizard. Answers "does the
/// shipped 88D2 register map actually work on my exact machine?" without
/// requiring the user to understand EC internals.
fn cmd_calibrate(cli: &Cli, seconds: u64, no_heat: bool) -> Result<()> {
    use std::io::{self, BufRead, Write as IoWrite};
    use std::time::Instant;

    dmi::ensure_root()?;
    let cfg = load_config(cli)?;
    let dmi = DmiInfo::read().unwrap_or_else(|_| dmi::DmiInfo {
        board_name: "<unreadable>".into(),
        product_name: "".into(),
        sys_vendor: "".into(),
    });
    let ec = Ec::open(Some(&cfg.ec_path));
    ec.check_available()?;

    println!("=== omen15-fan calibration wizard ===");
    println!(
        "Board detected: {} (expected 88D2) — {}\n",
        dmi.board_name,
        if dmi.matches_expected() { "OK" } else { "DIFFERENT, be extra careful" }
    );
    println!(
        "This will:\n\
         1) take a snapshot of all EC registers,\n\
         2) {} for {seconds}s so the stock BIOS fan curve ramps up on its own,\n\
         3) take a second snapshot and show you exactly which bytes changed,\n\
         4) optionally (only if you say yes) run a short, low, safe manual\n\
            fan-speed test so we can see whether the shipped register map\n\
            actually moves your fan.\n\
         Nothing here can brick the laptop — worst case the fan runs at the\n\
         wrong speed for a few seconds; `omen15-fan auto` or a reboot always\n\
         puts the BIOS back in charge.\n",
        if no_heat { "wait (you do the heating — e.g. run a game)" } else { "run a built-in CPU stress loop" }
    );

    let before = ec.read_all()?;

    if no_heat {
        println!("Do whatever makes the fan ramp up now (game, build, stress-ng...).");
        println!("Press Enter here once it's ramped up.");
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
    } else {
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        println!("Heating CPU with {n} busy threads for {seconds}s...");
        let stop = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::new();
        for _ in 0..n {
            let stop = stop.clone();
            handles.push(std::thread::spawn(move || {
                let mut x: u64 = 0;
                while !stop.load(Ordering::Relaxed) {
                    x = x.wrapping_mul(2654435761).wrapping_add(1);
                    std::hint::black_box(x);
                }
            }));
        }
        let start = Instant::now();
        while start.elapsed().as_secs() < seconds {
            print!(".");
            io::stdout().flush().ok();
            std::thread::sleep(Duration::from_secs(1));
        }
        stop.store(true, Ordering::Relaxed);
        for h in handles {
            let _ = h.join();
        }
        println!();
    }

    let after = ec.read_all()?;

    let mut diffs: Vec<(usize, u8, u8)> = Vec::new();
    for i in 0..ec::EC_SIZE {
        if before[i] != after[i] {
            diffs.push((i, before[i], after[i]));
        }
    }
    diffs.sort_by_key(|(_, b, a)| std::cmp::Reverse((*a as i16 - *b as i16).unsigned_abs()));

    println!("\n=== Registers that changed ({}) ===", diffs.len());
    if diffs.is_empty() {
        println!(
            "Nothing changed. Either the fan didn't ramp up during the test\n\
             (try a longer/heavier load), or this board reports fan state\n\
             somewhere `probe --watch` would catch over a longer window."
        );
    }
    for (off, b, a) in &diffs {
        let mark_read = *off as u8 == cfg.registers.fan1_read;
        let mark_ctrl = *off as u8 == cfg.registers.control;
        let tag = if mark_read {
            "  <-- matches configured fan1_read"
        } else if mark_ctrl {
            "  <-- matches configured control"
        } else {
            ""
        };
        println!("  0x{off:02X}: 0x{b:02X} -> 0x{a:02X}{tag}");
    }

    let configured_read_changed = diffs.iter().any(|(off, ..)| *off as u8 == cfg.registers.fan1_read);
    println!();
    if configured_read_changed {
        println!(
            "Good sign: the configured fan1_read register (0x{:02X}) changed during\n\
             load, consistent with the shipped 88D2 map.",
            cfg.registers.fan1_read
        );
    } else if !diffs.is_empty() {
        println!(
            "The configured fan1_read register (0x{:02X}) did NOT change, but other\n\
             bytes did. The strongest candidate(s) above may be your real fan-speed\n\
             register on this firmware — consider updating [registers] in\n\
             {} accordingly, then re-run calibrate.",
            cfg.registers.fan1_read, cli.config
        );
    }

    println!("\n=== Optional: live manual-control test ===");
    println!(
        "This will set the fan to a modest ~{} RPM for 5 seconds using the\n\
         CONFIGURED registers (control=0x{:02X}, target=0x{:02X}), then hand\n\
         control back to the BIOS automatically. Proceed? [y/N]",
        cfg.idle_rpm.max(2200),
        cfg.registers.control,
        cfg.registers.fan1_target
    );
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    if answer.trim().eq_ignore_ascii_case("y") {
        dmi.require_known_board(cli.force)?;
        let test_rpm = cfg.idle_rpm.max(2200).min(cfg.max_rpm);
        let raw = cfg.rpm_to_raw(test_rpm);
        ec.write_byte(cfg.registers.control, 1)?;
        ec.write_byte(cfg.registers.fan1_target, raw)?;
        println!("Set to ~{test_rpm} RPM, watching fan1_read for 5s:");
        for _ in 0..5 {
            std::thread::sleep(Duration::from_secs(1));
            let read = ec.read_byte(cfg.registers.fan1_read)?;
            println!("  fan1_read = 0x{:02X} (~{} RPM)", read, cfg.raw_to_rpm(read));
        }
        ec.write_byte(cfg.registers.control, 0)?;
        println!("Done — control handed back to the BIOS.");
        println!(
            "If fan1_read climbed toward ~{test_rpm} RPM and you heard/felt the fan\n\
             respond, the shipped register map works on your machine. If nothing\n\
             happened, see docs/CALIBRATION.md and try the candidate offsets above."
        );
    } else {
        println!("Skipped. Run `omen15-fan calibrate` again anytime, or test manually with `set`.");
    }

    Ok(())
}

fn cmd_set(cli: &Cli, percent: Option<u8>, rpm: Option<u32>) -> Result<()> {
    dmi::ensure_root()?;
    let cfg = load_config(cli)?;
    let dmi = DmiInfo::read()?;
    dmi.require_known_board(cli.force)?;

    let target_rpm = match (percent, rpm) {
        (Some(p), None) => (cfg.max_rpm as f32 * (p.min(100) as f32) / 100.0) as u32,
        (None, Some(r)) => r,
        (None, None) => anyhow::bail!("pass --percent or --rpm"),
        (Some(_), Some(_)) => unreachable!("clap conflicts_with enforces this"),
    };
    let target_rpm = target_rpm.min(cfg.max_rpm);
    let raw = cfg.rpm_to_raw(target_rpm);

    let ec = Ec::open(Some(&cfg.ec_path));
    ec.check_available()?;
    let r = &cfg.registers;
    ec.write_byte(r.control, 1)?;
    ec.write_byte(r.fan1_target, raw)?;
    if let Some(t2) = r.fan2_target {
        ec.write_byte(t2, raw)?;
    }
    println!(
        "manual mode on, target ~{} RPM (raw 0x{:02X})",
        cfg.raw_to_rpm(raw),
        raw
    );
    Ok(())
}

fn cmd_auto(cli: &Cli) -> Result<()> {
    dmi::ensure_root()?;
    let cfg = load_config(cli)?;
    let ec = Ec::open(Some(&cfg.ec_path));
    ec.check_available()?;
    ec.write_byte(cfg.registers.control, 0)?;
    println!("control handed back to BIOS");
    Ok(())
}

fn cmd_curve(cli: &Cli) -> Result<()> {
    dmi::ensure_root()?;
    let cfg = load_config(cli)?;
    let dmi = DmiInfo::read()?;
    dmi.require_known_board(cli.force)?;
    cfg.validate()?;

    let ec = Ec::open(Some(&cfg.ec_path));
    ec.check_available()?;
    let sensor = hwmon::TempSensor::discover(&cfg.hwmon_chip)
        .context("no CPU temperature sensor — check hwmon_chip in config")?;

    let running = Arc::new(AtomicBool::new(true));
    {
        let running = running.clone();
        let ec_path = cfg.ec_path.clone();
        let control_reg = cfg.registers.control;
        ctrlc::set_handler(move || {
            running.store(false, Ordering::SeqCst);
            // Best-effort immediate revert; the main loop also does this on
            // the way out, this just covers a slow shutdown.
            let _ = Ec::open(Some(&ec_path)).write_byte(control_reg, 0);
        })
        .context("setting signal handler")?;
    }

    let r = cfg.registers.clone();
    ec.write_byte(r.control, 1)
        .context("switching EC to manual control")?;
    println!(
        "omen15-fan curve running (chip={}, poll={}ms, max_rpm={}, idle_rpm={}). Ctrl+C to stop and hand back to BIOS.",
        sensor.chip_name(),
        cfg.poll_interval_ms,
        cfg.max_rpm,
        cfg.idle_rpm
    );

    let mut tracked_temp = sensor.read_max_c().unwrap_or(0.0);
    let mut last_rpm: Option<u32> = None;

    while running.load(Ordering::SeqCst) {
        match sensor.read_max_c() {
            Ok(t) => {
                if t > tracked_temp {
                    tracked_temp = t;
                } else if tracked_temp - t >= cfg.hysteresis_c {
                    tracked_temp = t;
                }
                let percent = curve::speed_percent_for_temp(&cfg.curve, tracked_temp);
                let target_rpm = if percent == 0 {
                    cfg.idle_rpm
                } else {
                    ((cfg.max_rpm as f32 * percent as f32 / 100.0) as u32).max(cfg.idle_rpm)
                }
                .min(cfg.max_rpm);

                if Some(target_rpm) != last_rpm {
                    let raw = cfg.rpm_to_raw(target_rpm);
                    if let Err(e) = ec.write_byte(r.fan1_target, raw) {
                        eprintln!("EC write failed: {e:#}");
                    } else {
                        if let Some(t2) = r.fan2_target {
                            let _ = ec.write_byte(t2, raw);
                        }
                        println!("temp={tracked_temp:.1}C -> {target_rpm} RPM");
                        last_rpm = Some(target_rpm);
                    }
                }
            }
            Err(e) => eprintln!("temp read failed: {e:#}"),
        }
        std::thread::sleep(Duration::from_millis(cfg.poll_interval_ms));
    }

    // Normal exit path (running flag cleared without the process being
    // killed) — hand control back to the BIOS.
    let _ = ec.write_byte(r.control, 0);
    println!("handed control back to BIOS, exiting");
    Ok(())
}

fn cmd_init_config(cli: &Cli) -> Result<()> {
    if std::path::Path::new(&cli.config).exists() {
        println!("{} already exists, not overwriting", cli.config);
        return Ok(());
    }
    dmi::ensure_root()?;
    let cfg = Config::default();
    cfg.save(&cli.config)?;
    println!("wrote default config to {}", cli.config);
    Ok(())
}

const SERVICE_UNIT: &str = include_str!("../systemd/omen15-fan.service");
const MODPROBE_CONF: &str = include_str!("../systemd/ec_sys.conf");
const MODULES_LOAD_CONF: &str = include_str!("../systemd/ec_sys.modules-load.conf");

fn cmd_service(action: &ServiceAction) -> Result<()> {
    dmi::ensure_root()?;
    match action {
        ServiceAction::Install => {
            std::fs::write("/etc/modprobe.d/omen15-fan-ec_sys.conf", MODPROBE_CONF)?;
            std::fs::write(
                "/etc/modules-load.d/omen15-fan-ec_sys.conf",
                MODULES_LOAD_CONF,
            )?;
            std::fs::write("/etc/systemd/system/omen15-fan.service", SERVICE_UNIT)?;
            Config::default().save(config::DEFAULT_CONFIG_PATH).ok();
            run(["systemctl", "daemon-reload"])?;
            run(["modprobe", "ec_sys", "write_support=1"])?;
            println!(
                "installed. Review {} then run:\n  sudo systemctl enable --now omen15-fan.service",
                config::DEFAULT_CONFIG_PATH
            );
        }
        ServiceAction::Uninstall => {
            run(["systemctl", "disable", "--now", "omen15-fan.service"]).ok();
            let _ = std::fs::remove_file("/etc/systemd/system/omen15-fan.service");
            let _ = std::fs::remove_file("/etc/modprobe.d/omen15-fan-ec_sys.conf");
            let _ = std::fs::remove_file("/etc/modules-load.d/omen15-fan-ec_sys.conf");
            run(["systemctl", "daemon-reload"])?;
            println!("service removed (config and /opt left in place)");
        }
    }
    Ok(())
}

fn run<const N: usize>(args: [&str; N]) -> Result<()> {
    let status = std::process::Command::new(args[0])
        .args(&args[1..])
        .status()
        .with_context(|| format!("running {args:?}"))?;
    if !status.success() {
        anyhow::bail!("{args:?} exited with {status}");
    }
    Ok(())
}
