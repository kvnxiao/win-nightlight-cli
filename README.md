# Windows Night Light CLI and Rust libraries

## Overview

Control Windows 11 Night Light from a terminal or a Rust application. This repository has a
command-line app and two Rust libraries:

- `win-nightlight-cli` installs `wnl.exe` to turn Night Light on or off, set its color temperature,
  and configure its schedule.
- `win-nightlight-lib` lets Rust applications read and write Night Light settings and state.
- `win-nightlight-watch` notifies Rust applications when registry values change so they can reread
  Night Light data.

**Tested on Windows 11 24H2 (OS build 26100) and build 26300.** Older Windows versions may
store Night Light differently.

## Table of contents

- [Overview](#overview)
- [Installation](#installation)
- [`win-nightlight-cli`: `wnl.exe` usage](#win-nightlight-cli-wnlexe-usage)
- [`win-nightlight-lib`: Rust API](#win-nightlight-lib)
- [`win-nightlight-watch`: registry notifications](#win-nightlight-watch)

## Installation

With Rust and Cargo installed on Windows, install the `wnl.exe` command from crates.io:

```shell
cargo install --locked win-nightlight-cli
```

Run it as `wnl` in your terminal. To see the available commands:

```shell
wnl --help
```

## `win-nightlight-cli`: `wnl.exe` usage

`on`, `off`, and `toggle` change only whether Night Light is on now; they do not change the
schedule. `temp` sets the color temperature and preserves the schedule and on/off state. These
commands print nothing on success. Temperatures use kelvin, from 1200K to 6500K.

```shell
wnl on
wnl off
wnl toggle
wnl temp 3400
```

### Command reference

```shell
Turn Windows 11 Night Light on and off and configure its schedule and color temperature

Usage: wnl.exe <COMMAND>

Commands:
  status       Show whether Night Light is on, the color temperature, and the schedule
  on           Turn Night Light on until you or the schedule turn it off
  off          Turn Night Light off until you or the schedule turn it on
  toggle       Turn Night Light on if it is off, or off if it is on
  temp         Set the color temperature
  schedule     Set the schedule that turns Night Light on and off
  completions  Print a shell completion script
  help         Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### `wnl status`

```shell
$ wnl status
active:            no
transition cause:  manual
last transition:   2026-03-26 05:04:27 EDT
state modified:    2026-03-26 05:04:29 EDT
color temperature: 3426K
schedule:          off
set hours:         21:00 to 07:00
sunset to sunrise: 19:17 to 06:38
settings modified: 2026-10-04 01:55:59 EDT
```

Times are 24-hour local time. `transition cause` is `manual` when a person or program last turned
Night Light on or off, and `scheduled` when the schedule did.

### `wnl schedule`

```shell
Usage: wnl.exe schedule <COMMAND>

Commands:
  off     Stop turning Night Light on and off on a schedule
  sunset  Turn Night Light on at sunset and off at sunrise
  hours   Turn Night Light on and off at set times
  help    Print this message or the help of the given subcommand(s)
```

`schedule hours` accepts `--start HH:MM` and `--end HH:MM` in 24-hour time and keeps the stored
time for an omitted option. `schedule off` keeps the schedule type and times, and does not change
whether Night Light is on. The command prints the new schedule and, when the schedule is on,
whether the current time is inside its window:

```shell
$ wnl schedule hours --start 21:00 --end 07:00
schedule set to set hours, 21:00 to 07:00; the current time is inside this window
$ wnl schedule sunset
schedule set to sunset to sunrise, 19:17 to 06:38; the current time is outside this window
$ wnl schedule off
schedule turned off
```

### Shell completions

`wnl completions <SHELL>` prints a completion script for `bash`, `elvish`, `fish`, `powershell`, or
`zsh`. For PowerShell, add this line to `$PROFILE`:

```powershell
wnl completions powershell | Out-String | Invoke-Expression
```

## `win-nightlight-lib`

Use the library to control Night Light from a Rust application. Add it to your project:

```shell
cargo add win-nightlight-lib
```

```rust
use win_nightlight_lib::ColorTemperature;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::RefreshPolicy;
use win_nightlight_lib::Schedule;

let nightlight = Nightlight::new();
nightlight.set_active(true)?;

let temperature: ColorTemperature = "3400K".parse()?;
nightlight.set_color_temperature(temperature, RefreshPolicy::Automatic)?;
nightlight.update_settings(|settings| settings.set_schedule(Schedule::SunsetToSunrise))?;
```

The library reads and writes the two registry values that store Night Light:

- `Settings`: the schedule mode and times, the color temperature, and the sunset and sunrise times
  Windows computes.
- `State`: whether Night Light is on now, what caused the last change, and when it happened.

`Nightlight` is the entry point. Its temperature operation can read settings and state but writes
only settings. On/off operations write only state and preserve the schedule.

### Temperature refresh

Preview requests that Windows temporarily apply the selected color temperature, even while Night
Light is off. Windows Settings uses it while adjusting the temperature slider.
`Nightlight::set_color_temperature` takes a `RefreshPolicy`. `Automatic`, used by `wnl temp`,
selects the refresh behavior from the current settings and state:

- If preview is already active, it preserves preview and stores a changed temperature.
- Otherwise, if Night Light is active, it writes the temperature with preview enabled, then rereads
  settings and clears preview. It repeats this sequence even when the temperature is unchanged.
- If Night Light is inactive, it stores a changed temperature without starting preview.

`Always` writes the requested temperature with preview enabled, then rereads settings and clears
preview on every call. This applies even when the temperature already matches or preview is already
active. It does not read or write the on/off state, so it also requests preview while Night Light is
off. Both policies preserve schedule settings. The return value reports whether settings were
written; a successful `Always` call returns `true`.

The writes run without a sleep. Each write first adds two seconds to the timestamp in the decoded
`Settings` or `State` record, saturating at `u64::MAX`. It then uses the later of that result and the
current Unix time.
`write_settings` and `write_state` use the record supplied by the caller. Temperature updates read
settings first and reread them for cleanup. Rapid refreshes can advance timestamps ahead of the
clock.

Avoid overlapping temperature calls or using the Night Light slider in Windows Settings during a
call. Cleanup rereads settings to preserve intervening changes, including a newer temperature, but
registry updates are not atomic with other writers. Successful writes do not confirm a display update or guarantee that
another writer has not changed the requested values.

If three cleanup attempts fail, `ErrorKind::PreviewCleanup` reports that the temperature was
written and preview may remain active. A later call with `Always` repeats the complete refresh,
including when the requested temperature is unchanged. `Automatic` preserves preexisting preview
and therefore does not provide that recovery contract.

`update_settings` and `write_settings` write at most once without adding this preview sequence.

### Types

| Type                           | Purpose                                                                 |
| :----------------------------- | :---------------------------------------------------------------------- |
| `Nightlight`                   | Reads, writes, and updates the settings and state values.               |
| `RefreshPolicy`               | Selects conditional refresh or a complete refresh on every temperature call. |
| `Settings`                     | Schedule and color temperature; `from_bytes` and `to_bytes` round-trip. |
| `State`                        | Whether Night Light is on, the transition cause, and its time.          |
| `ColorTemperature`             | Validated color temperature from 1200K to 6500K.                        |
| `TimeOfDay`                    | 24-hour `HH:MM` time with a midnight-wrapping window check.             |
| `Schedule`                     | Schedule change passed to `Settings::set_schedule`.                     |
| `ScheduleMode`, `ScheduleKind` | Effective schedule mode, and the type kept while the schedule is off.   |
| `TransitionCause`              | Whether the last on/off change was manual or scheduled.                 |
| `Error`, `ErrorKind`, `Result` | Errors classified as not found, registry, decode, encode, input, or preview cleanup.     |

### Registry format

Settings and state are stored as `REG_BINARY` values named `Data` under:

- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings`
- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate`

The format is Microsoft Bond CompactBinary v1 inside a CloudStore envelope. Microsoft does not
document it; the library follows reverse-engineered schemas. It preserves fields it does not model,
except unknown fields inside time-of-day records, which it discards during decoding.
See [`docs`](docs/) for the format details.

## `win-nightlight-watch`

Use the watcher to wait for registry changes from a Rust application. Add it to your project:

```shell
cargo add win-nightlight-watch
```

`Watcher` observes value changes in one existing registry key under `HKEY_CURRENT_USER`.
Use `SETTINGS_KEY_PATH` and `STATE_KEY_PATH` from `win-nightlight-lib` to watch Night Light.
The watcher opens the key for notifications and creates an event; it does not start a thread.

Create each watcher before reading the initial data. After a successful wait, call `rearm()`
once before rereading through `Nightlight`. Do not rearm after a timeout. Notifications can
combine multiple writes; they tell the application to read the current data.

Use `wait_timeout(Duration)` for a blocking wait, or `AsHandle` / `AsRawHandle` to integrate
with a native Windows wait or message loop. Each watcher has one notification consumer.
Drop it after any rearm error; recovery by reopening the key belongs to the application.

See [change notifications](docs/change-notifications.md) for lifecycle, timeout, and error
contracts, and the [crate documentation](https://docs.rs/win-nightlight-watch) for a compiled
usage example.
