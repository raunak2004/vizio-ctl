# vizio-ctl

An unofficial command-line tool and local web UI for controlling a Vizio SmartCast soundbar over your home network.

> **Disclaimer (draft only; review required by Legal, Compliance, or the relevant control owner before relying on it):**
> This project is not affiliated with, endorsed by, or sponsored by Vizio. "Vizio" and "SmartCast" are trademarks of their respective owners. It uses the soundbar's local network API, which is undocumented and may change. Use it at your own risk.

## What it can do

- Power on/off, volume up/down, mute
- Sound settings: bass, treble, balance, center, surround volume, subwoofer, AV delay, EQ, VRT-X, surround sound, night mode
- Input selection, sleep timer, eco power, power LED, TV remote, language, country, time zone, daylight savings
- Actions such as speaker tests and input auto-detect
- A device information view (model, firmware, Cast version, audio format, network details)
- A small web page that does all of the above from a browser

## Tested on

| Item | Value |
|---|---|
| Model | SB3651-E6 |
| Firmware | V3.1.3.1 |
| Cast firmware | 1.36.145853 |

Other Vizio soundbars and speakers that expose the same SmartCast API may work, but are untested.

### What has been verified

Verified on the device above: `status`, `on`, `off`, `vol-up`, `vol-down`, `mute`, and setting bass. The web UI has been confirmed to load from another device on the same network.

Implemented but **not yet verified**: the other settings, input selection, the actions, the `info` command, and the web UI's newer controls. Reports and fixes are welcome (see [Contributing](#contributing)).

## Install

You need a [Rust toolchain](https://rustup.rs). Then:

```bash
cargo install --git https://github.com/raunak2004/vizio-ctl
```

Make sure `~/.cargo/bin` is on your `PATH`.

Or build from a clone:

```bash
git clone https://github.com/raunak2004/vizio-ctl
cd vizio-ctl
cargo build --release
./target/release/vizio-ctl --help
```

On Linux the build needs OpenSSL development headers (for example `libssl-dev` on Debian/Ubuntu).

## Find your soundbar's IP address

Check your router's list of connected devices, or look for a name like `VIZIOCastAudio…`. Giving the soundbar a fixed address (a DHCP reservation) in your router keeps the address from changing.

## Usage

Every command takes `--host <IP>`:

```bash
vizio-ctl --host 192.168.1.50 status
vizio-ctl --host 192.168.1.50 on
vizio-ctl --host 192.168.1.50 vol-up 3
vizio-ctl --host 192.168.1.50 mute
```

Run `vizio-ctl --help` or `vizio-ctl <command> --help` for details.

| Command | What it does |
|---|---|
| `status` | Power, volume, mute and all current settings |
| `on`, `off` | Power the soundbar on or off |
| `vol-up [N]`, `vol-down [N]` | Change volume by N steps (default 1) |
| `mute` | Toggle mute |
| `bass`, `treble`, `balance [LEVEL]` | Show, or set, a level from -12 to 12 |
| `set <name> [value]` | Show or set any setting (see below) |
| `action <name>` | Run an action such as a speaker test |
| `preset <music\|movie> [--save]` | Apply a sound preset, or save the current sound settings as it |
| `info [--reveal]` | Show model, firmware and network information |
| `serve` | Start the local web UI |

### Settings for `set`

| Name | Values |
|---|---|
| `bass`, `treble`, `balance`, `center`, `surround-volume`, `subwoofer` | -12 to 12 |
| `av-delay` | 0 to 8 |
| `eq` | Movie, Music, Direct |
| `vrt-x`, `surround-sound`, `night-mode` | On, Off |
| `input` | Inputs the soundbar reports, for example HDMI-ARC, Optical, Bluetooth |
| `sleep-timer`, `eco-power`, `power-led`, `tv-remote`, `language`, `country`, `timezone`, `dst` | Choices are read from the soundbar |

```bash
vizio-ctl --host 192.168.1.50 set eq Music
vizio-ctl --host 192.168.1.50 set subwoofer -2
vizio-ctl --host 192.168.1.50 set input Optical
```

### Presets

Two presets, `music` and `movie`, apply a set of sound settings (EQ, surround sound, VRT-X, bass, treble, center, surround volume, subwoofer) in one step:

```bash
vizio-ctl --host 192.168.1.50 preset music
vizio-ctl --host 192.168.1.50 preset movie
```

They start with built-in values, which are only a starting point; sound is subjective and room-dependent. To make a preset your own, set the soundbar the way you like it, then save it:

```bash
vizio-ctl --host 192.168.1.50 preset music --save
```

Saved presets are stored in a JSON file in your config folder (`presets.json` next to the tool's other config, for example under `~/Library/Application Support/vizio-ctl/` on macOS). The soundbar itself does not store them. The web UI has the same Music and Movie buttons, each with a "Save current" button.

### Actions

`speaker-test-auto`, `speaker-test-left`, `speaker-test-center`, `speaker-test-right`, `speaker-test-left-surround`, `speaker-test-right-surround`, `speaker-test-subwoofer`, `input-auto-detect`, `bluetooth-pair`, `subwoofer-pair`, `check-update`, `power-cycle`.

Factory reset, SoftAP and remote-learning actions are intentionally not included.

### Information and privacy

`info` hides identifying values (serial number, MAC addresses, SSID, IP, ESN) unless you pass `--reveal`. Avoid pasting revealed output into public issues.

## Web UI

```bash
vizio-ctl --host 192.168.1.50 serve
```

Then open <http://127.0.0.1:8080>. Use `--port` to change the port.

### Use it from another device on your network

By default the server listens only on `127.0.0.1`, so only your own computer can use it. To open the page from a phone, tablet or another computer on the same network:

1. Find your computer's address on the network. On macOS Wi-Fi:

   ```bash
   ipconfig getifaddr en0
   ```

   On Linux, `hostname -I` lists addresses.

2. Start the server bound to that address (use your own address in place of `192.168.1.20`):

   ```bash
   vizio-ctl --host 192.168.1.50 serve --bind 192.168.1.20
   ```

   Binding to your computer's address listens on that network interface only. `--bind 0.0.0.0` listens on every interface, which is broader.

3. On the other device, open `http://192.168.1.20:8080`.

Troubleshooting:

- **Allow incoming connections** if your operating system's firewall asks the first time.
- **Same network:** the other device must be on the same network as your computer, not a guest network. Some routers isolate Wi-Fi devices from each other ("AP isolation" or "client isolation"), which blocks this.
- **The server must keep running** and the computer must stay awake.
- **Address changes:** if your computer's address changes, the link stops working. A DHCP reservation for it in your router keeps it fixed.

### Security

**The web UI has no login.** While it is bound to your network, anyone on that network can open the page and control the soundbar. Only do this on a network you trust.

- Do not forward the port on your router or expose it to the internet.
- Stop the server (Ctrl+C) when you are done; running it again without `--bind` returns to computer-only access.
- The server rejects requests that lack a custom header, which stops other websites from driving it through your browser. This does not protect against devices on your own network.

## How it works

Soundbars in this family expose a local HTTPS API on port 9000 (some models use 7345) with a self-signed certificate, so the tool does not verify that certificate. Settings are read from the device's own menu tree, and the allowed choices come from the device. This particular soundbar accepts commands without pairing. The `pair` command exists for devices that require a PIN, but it has not been verified.

## Limitations

- No absolute volume control: volume changes by steps.
- The API is unofficial and undocumented; behavior may differ by model and firmware.
- Cast playback (playing audio from a URL or file) is not implemented.

## Contributing

Issues and pull requests are welcome, especially:

- Test results from other models and firmware versions
- Fixes for the unverified features listed above

When filing an issue, include the model name and firmware version, and **remove serial numbers, MAC addresses, SSIDs, IP addresses and any auth tokens** from anything you paste.

```bash
cargo fmt
cargo clippy
cargo build
```

## License

No license has been chosen yet. Until one is added, the code is all rights reserved by the author.
