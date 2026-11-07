# BT-W5 Control

A minimal, private replacement for the Creative App, for one device: the Creative BT-W5
Bluetooth audio dongle (product code AYGO4, USB 041E:3130). Rust + Slint, a single exe, no
telemetry, no account, no background process.

## What it does

- **Device**: model, serial number, firmware version, audio driver version, current headphone
  and microphone formats.
- **Check for updates**: checks Creative's server for newer firmware and audio driver and shows
  the result. It never installs them.
- **Download firmware**: downloads Creative's firmware installer for the current release
  (HTTPS), verifies size and CRC32 from Creative's metadata and the Authenticode signature,
  saves it to Downloads and shows it in Explorer.
- **Download audio driver**: the same for Creative's USB audio driver installer
  (`CTUSBAudioDriverInstaller.exe`), verified against the MD5 in Creative's driver list.
- Nothing downloaded is ever run.
- **Bluetooth**: paired devices with connect/disconnect, pairing a new device (search, then
  pair), and "Reset Bluetooth list" (forget all, after confirmation).

Fixed behaviour, not configurable:

| Setting | How |
| --- | --- |
| Language | Windows display language (Dutch or English) |
| Theme | Follows the Windows app theme (Slint Fluent style) |
| Auto reconnect | The dongle's own setting is read on every connection and at logon, and switched on when it is off |
| Run at startup | `HKCU\...\Run` → `bt-w5-control.exe --startup`, re-registered on every launch |
| Effects off, EQ off | Windows audio enhancements off (`PKEY_AudioEndpoint_Disable_SysFx = 1`) on both BT-W5 endpoints, which bypasses Creative's APO entirely |
| Playback: headphones stereo | `PKEY_AudioEndpoint_PhysicalSpeakers = 3` (front L/R) |
| Headphones 24-bit / 96 kHz | Set through `IPolicyConfig` if the endpoint supports it |
| Microphone highest quality | Highest exclusive-mode format the endpoint supports (16-bit / 48 kHz on the BT-W5) |

`--startup` applies the audio defaults (waiting up to a minute for the dongle), makes sure auto
reconnect is on, and exits;
there is no resident process. Windows keeps these settings per endpoint, so they survive
without the app.

## Command line

| Flag | Effect |
| --- | --- |
| *(none)* | Opens the window, registers autostart, applies the audio defaults |
| `--startup` | Applies the audio defaults and exits (used at logon) |
| `--dry-run` | Opens the window but writes nothing: no autostart, no audio changes |
| `--probe` | Read-only diagnostics on the console (debug build): endpoints, formats, HID replies, update checks |
| `--uninstall` | Removes the autostart entry and turns audio enhancements back on |

## Install with Scoop

From the private bucket (needs `scoop config gh_token`, see the bucket's README):

```powershell
scoop install roosmsg/bt-w5-control
```

Start it once from the Start menu; that registers the logon entry, pointing at Scoop's
`current` folder so it survives updates. `scoop uninstall bt-w5-control` removes the entry.

## Before first use

Close and uninstall the Creative App. Both apps talking to the dongle's HID channel at the same
time interleave replies; the window shows a warning while `Creative.App.exe` runs. Uninstalling
the Creative App leaves the audio driver and APO in place.

## Build

Toolchain: rustup with `stable-x86_64-pc-windows-gnu` + MinGW (scoop `rustup-gnu`, `mingw`).
Scoop sets `CARGO_HOME`/`RUSTUP_HOME` as user variables; shells started before the install need:

```powershell
$env:CARGO_HOME = "$env:USERPROFILE\scoop\persist\rustup-gnu\.cargo"
$env:RUSTUP_HOME = "$env:USERPROFILE\scoop\persist\rustup-gnu\.rustup"
$env:PATH = "$env:CARGO_HOME\bin;$env:USERPROFILE\scoop\apps\mingw\current\bin;$env:PATH"
cargo +stable clippy --all-targets
cargo +nightly fmt --all
cargo test
cargo build --release
```

`scripts\install.ps1` builds the release exe, copies it to `%LOCALAPPDATA%\Programs\BT-W5 Control`
and adds a Start menu shortcut (`-Launch` starts it), for use without Scoop.

## Release

1. Raise `version` in `Cargo.toml`.
2. `scripts\package.ps1` builds `dist\bt-w5-control-<version>-windows-x64.zip` (just the exe; icon,
   version info and manifest are embedded; `scripts\build-icon.ps1` regenerates `ui\icon.ico`).
3. `gh release create v<version> dist\bt-w5-control-<version>-windows-x64.zip`
4. In the bucket: `.\bin\publish-manifest.ps1 -App bt-w5-control`

## Layout

| File | Role |
| --- | --- |
| `src/protocol.rs` | Creative HID frames (`5A cmd len payload`, report id 3, 65 bytes) and parsers, unit-tested against frames captured from a BT-W5 |
| `src/device.rs` | HID worker thread: owns the handle, matches replies, forwards state and scan events |
| `src/audio.rs` | Endpoint discovery, `IPolicyConfig` (Vista variant), defaults |
| `src/updates.rs` | Update checks and verified installer download |
| `src/winutil.rs` | Dialogs, single instance, language, signature check, process launch, autostart |
| `src/i18n.rs` | English and Dutch text |
| `ui/app.slint` | The window |
