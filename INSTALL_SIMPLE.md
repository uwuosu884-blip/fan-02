# Simple install guide (no Linux experience needed)

This is the plain-language version. Just copy each grey box into your
terminal, one at a time, and read the one line of explanation above it.
Press **Enter** after pasting each box.

You'll be asked for your password sometimes — that's normal, it's what
`sudo` does (it means "do this as the administrator"). Typing a password in
Linux terminals shows **nothing at all**, not even dots — just type it and
press Enter.

---

## Step 0 — Will this break anything?

No. Short answer, see the FAQ at the bottom for the long one. Worst case the
fan runs too slow/fast for a few seconds — never dangerous, and one command
(`omen15-fan auto`) or a reboot always gives control back to your BIOS.

## Step 1 — Get the code onto your computer

Open a terminal and paste:

```bash
git clone https://github.com/uwuosu884-blip/fan-02.git
cd fan-02/omen15-fan
```

*(This downloads the project into a folder called `fan-02` and moves you
into it.)*

## Step 2 — Install Rust (the programming language this is written in)

This only needs to happen once, and keeps itself out of the way:

```bash
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
source "$HOME/.cargo/env"
```

## Step 3 — Build the program

This turns the source code into a program you can run. Takes under a minute.

```bash
cargo build --release
```

When it finishes you'll see `Finished release [...]` — that means it worked.

## Step 4 — Install it on your system

```bash
sudo install -m755 target/release/omen15-fan /usr/local/bin/omen15-fan
sudo omen15-fan init-config
```

*(This copies the program somewhere your whole system can find it, and
writes its settings file to `/etc/omen15-fan/config.toml`.)*

## Step 5 — Turn on the piece that lets it talk to the fan

```bash
sudo omen15-fan service install
```

## Step 6 — Check it actually works on YOUR laptop (important, don't skip)

This is the one real "test" step. It's interactive and explains itself as it
goes:

```bash
sudo omen15-fan calibrate
```

It will:
1. Show your laptop's board ID (should say `88D2`).
2. Spin your CPU up for 20 seconds (fans will get loud — that's expected,
   it's just making your normal BIOS fan curve kick in so we can watch it).
3. Print which fan-related settings changed — you don't need to understand
   these, just check it says things look consistent (it will tell you).
4. Ask **"Proceed? [y/N]"** — type `y` and press Enter to let it try
   controlling the fan directly for 5 seconds at a gentle speed, so you can
   hear whether it responds.

If the fan audibly responds during that 5-second test — it works. If
nothing happens, don't worry, nothing is broken; paste me what it printed
and I'll help adjust the settings for your exact machine.

## Step 7 — Turn on automatic fan control

Once step 6 looks good:

```bash
sudo systemctl enable --now omen15-fan.service
```

Your fans are now controlled by the included temperature curve instead of
the stock BIOS. To check it's alive at any time:

```bash
sudo omen15-fan status
```

## If you ever want to undo everything

```bash
sudo systemctl disable --now omen15-fan.service
sudo omen15-fan auto
```

That instantly gives control back to the BIOS, exactly like before you
installed anything.

---

## FAQ

**Is there a command to calibrate the fans?**
Yes — `sudo omen15-fan calibrate` (step 6 above). It's a guided, interactive
wizard; you don't need to read any technical docs to use it.

**Will my PC explode / get damaged?**
No. A few reasons:
- This tool only ever sets a fan *speed* — it can't turn fans off entirely
  in a way the BIOS can't override, and it can't disable your CPU/GPU's own
  built-in thermal shutdown protection, which exists completely separately
  from fan software and will shut things down before real heat damage.
- The program has a hard speed ceiling (`max_rpm`, default 5500 RPM) baked
  in — it will never command the fan faster than that, no matter what.
- If anything behaves oddly, `sudo omen15-fan auto` or simply **restarting
  the laptop** immediately restores normal BIOS-controlled behavior —
  nothing is permanently changed in firmware.
- The worst realistic outcome is "fans too quiet while gaming" (temps climb,
  CPU/GPU throttles itself down, annoying but not damaging) or "fans louder
  than necessary while idle" — not hardware failure.

**What if `calibrate` says nothing changed, or the live test doesn't make
the fan respond?**
That means this specific firmware revision of board 88D2 uses different
register numbers than the ones this tool assumes by default. It's not
dangerous, it just means fan control won't take effect yet. Share the
`calibrate` output with me and I'll adjust the register settings in
`config.toml` for your exact machine.

**Do I need to run `calibrate` every time?**
No, just once, to confirm it works. After that, `systemctl enable --now
omen15-fan.service` keeps it running automatically, including after
reboots.
