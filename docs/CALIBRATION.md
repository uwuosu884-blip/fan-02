# Calibrating omen15-fan on your en1003ur

The register offsets shipped in `config/config.toml` (`control=0x15`,
`fan1_target=0x19`, `fan1_read=0x11`) come from third-party projects (notably
[pankha](https://github.com/VulnX/pankha)) that report them working on board
**88D2**, the same board id your HP Omen 15-en1003ur reports. That's a good
starting point, **not a guarantee** — other tools (OmenMon's model database)
show that different BIOS/EC firmware revisions of the same board id can use
different offsets. Do this calibration pass once before trusting `set` or
`curve` on real hardware.

## 0. Prerequisites

```bash
sudo modprobe ec_sys write_support=1
sudo omen15-fan status
```

`status` should show your board as `88D2` and confirm the EC file exists. If
`ec_sys` won't load with write support, check `dmesg` — some kernels build it
read-only; you'd need `acpi_call` or a custom driver instead (see README's
"If ec_sys doesn't work" section).

## 1. Confirm the control + read registers without writing anything risky

```bash
sudo omen15-fan probe --watch
```

While it's running:
1. Let the laptop idle for ~10s — note the baseline bytes.
2. Open a CPU-heavy task (e.g. `stress-ng --cpu 4 --timeout 30s`) so the
   stock BIOS fan curve ramps the fan up on its own. Watch which offsets
   change as RPM goes up — that's almost certainly your **read/report**
   register(s). If it changes in two different places as CPU vs GPU heat up
   independently, you likely have two separate fans/registers.
3. In the BIOS or HP's own tool (if dual-booting Windows) or by holding the
   OMEN key (if your model maps it to "max fan"), toggle max fan / silent
   mode and see which byte flips between 0 and non-zero — that's the
   **control** register.

Compare what you see against `0x11` (read) and `0x15` (control). If they
match, you're aligned with the shipped config. If not, edit
`/etc/omen15-fan/config.toml`'s `[registers]` section to the offsets you
actually observed.

## 2. Confirm the write/target register — small, reversible steps

Do this only after step 1 looks consistent.

```bash
sudo omen15-fan set --rpm 2500   # modest bump, should be audible but mild
sudo omen15-fan status           # fan1_read should climb toward ~2500 RPM
sudo omen15-fan auto             # give control back to BIOS immediately
```

If the fan doesn't respond, or ramps to something wildly different than
requested, stop and re-check `probe --watch` while running `set` in another
terminal — you should see `fan1_target` (0x19 by default) change exactly when
you call `set`, and `fan1_read` follow a second or two later.

## 3. Two fans?

The Omen 15-en1xxx has a CPU fan and a GPU fan. Pankha's documented 88D2
register set only lists one control/target/read triplet, which either means:
(a) both fans share the same target register and move together, or
(b) only one fan's registers were confirmed and the second is still unknown.

To find out: run a GPU-only load (e.g. a short game or `glmark2`) vs a
CPU-only load (`stress-ng --cpu`) while running `probe --watch`, and see if
different offsets move independently. If so, set `fan2_target`/`fan2_read` in
the config to the offsets you found.

## 4. Once confirmed

- Tune `max_rpm` to your fans' real maximum (don't exceed what the stock BIOS
  ever drives them to — that's your safe ceiling).
- Adjust the `[[curve]]` points to taste.
- Install the service: `sudo omen15-fan service install && sudo systemctl
  enable --now omen15-fan.service`.
- Watch `journalctl -u omen15-fan -f` for a while under normal use before
  trusting it unattended.

## Safety notes

- `omen15-fan` always writes `max_rpm` as a hard ceiling, clamps all inputs,
  and reverts to BIOS auto-control (`control=0`) on Ctrl+C, systemd stop, or
  normal exit.
- If anything looks wrong (fan stuck, no response, erratic RPM), run
  `sudo omen15-fan auto` immediately, or `sudo systemctl stop
  omen15-fan.service`, or just reboot — EC state is not persistent across a
  power cycle beyond what the BIOS itself restores.
