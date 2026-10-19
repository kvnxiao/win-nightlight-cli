# Windows Night Light CLI and Rust libraries

## Overview

Control Windows 11 Night Light from a terminal or a Rust application. This repository has a
command-line app and two Rust libraries:

- `win-nightlight-cli` installs `wnl.exe` to turn Night Light on or off, set its color temperature,
  and configure its schedule.
- `win-nightlight-lib` lets Rust applications read and write Night Light settings and state.
- `win-nightlight-watch` waits for Night Light changes and returns decoded settings or state.

**Tested on Windows 11 24H2 (OS build 26100) and build 26300.** Older Windows versions may
store Night Light differently.

## Table of contents

- [Overview](#overview)
- [Installation](#installation)
- [`win-nightlight-cli`: `wnl.exe` usage](#win-nightlight-cli-wnlexe-usage)
- [`win-nightlight-lib`: Rust API](#win-nightlight-lib)
- [`win-nightlight-watch`: change snapshots](#win-nightlight-watch)

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

`on`, `off`, and `toggle` change whether Night Light is on now; they do not change the schedule.
`temp` sets the color temperature and preserves the schedule and on/off state. While Night Light is
on, the screen changes right away. While it is off, `temp` stores the temperature, and the screen
shows it when Night Light turns on. Temperatures use kelvin, from 1200K (warmest) to 6500K (no
tint), with or without the `K`. These commands print nothing on success.

`on`, `off`, `toggle`, `temp`, and `schedule` also turn off preview. Preview is the temporary tint
shown while the user holds the Strength slider in Windows Settings, even when Night Light is off.
If a program exits during a preview, the tint can remain on; these commands clear it. Cleanup writes
settings only when preview was on.

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

`schedule hours` accepts `--start HH:MM` and `--end HH:MM` in 24-hour time and keeps the stored time
for an omitted option. `schedule off` keeps the schedule type and times, and does not change whether
Night Light is on. Each `schedule` command prints the new schedule and, when the schedule is on,
whether the current time is inside its window:

```shell
$ wnl schedule hours --start 21:00 --end 07:00
schedule set to set hours, 21:00 to 07:00; the current time is outside this window
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
use win_nightlight_lib::Schedule;

let nightlight = Nightlight::new();
nightlight.set_active(true)?;

let temperature: ColorTemperature = "3400K".parse()?;
nightlight.apply_color_temperature(temperature)?;
nightlight.update_settings(|settings| settings.set_schedule(Schedule::SunsetToSunrise))?;
```

The library reads and writes the two registry values that store Night Light. It decodes them into
two types:

- The settings value, as `Settings`: the schedule mode and times, the color temperature, the
  preview flag, and the sunset and sunrise times Windows computes.
- The state value, as `State`: whether Night Light is on now, what caused the last change, and
  when it happened.

`Nightlight` is the entry point. Its temperature methods can read settings and state but write
only settings. Its on/off methods write only state and preserve the schedule.

### Change the color temperature

Choose a method based on the change you want to make:

| Change | Method |
| :-- | :-- |
| One new temperature | `apply_color_temperature(t)`; also used by `wnl temp` |
| Store a temperature or make a long, gradual change | `set_color_temperature(t)` |
| Make a short transition | `start_preview()`, then `preview_color_temperature(t)` for each step, then `end_preview()` |

**Preview** makes Windows show the stored temperature even while Night Light is off. Windows
Settings normally enables it while the user holds the Strength slider.

- `apply_color_temperature` briefly turns preview on when Night Light is on. It waits 50 ms, then
  turns preview off. This happens even when the temperature already matches. When Night Light is
  off, it stores the temperature with preview off, and skips the write if both already match.
- `set_color_temperature` makes one write without changing preview. It skips the write if the
  temperature already matches.
- The preview methods each write once, even if nothing changes. They do not sleep; the caller
  controls how long preview stays on.

`apply_color_temperature` and the preview methods return whether preview was on before the call.
The 50 ms wait blocks the calling thread. If `apply_color_temperature` fails to turn preview off
in three attempts, it returns `ErrorKind::PreviewCleanup`; preview may remain on. Retrying repeats
the whole sequence.

For lower-level changes, `update_settings` and `write_settings` write at most once. They store the
preview flag from the resulting `Settings` record without adding a preview sequence.

Avoid overlapping updates: another writer can change settings between a read and the subsequent
write. `write_settings` writes the record supplied by the caller. A successful registry write does
not confirm that the display changed.

See the [color temperature transitions guide](docs/color-temperature-transitions.md) for transition examples, write rates, cleanup, and interactions with the slider.

### Settings timestamps

Keep one `Nightlight` handle for the application's lifetime. For an application that makes gradual
changes, use `Nightlight::new().with_startup_margin()` to reduce the risk of ignored writes after
the application restarts.

Each settings or state write has a timestamp in whole seconds. The library chooses the latest of:

- The current time.
- Two seconds after the timestamp in the record being written.
- Two seconds after the highest timestamp that handle has written to the same registry value.

Frequent writes therefore move timestamps ahead of real time. The handle remembers its own highest
timestamp, so it can keep increasing it even if the slider stores a lower one. A new handle loses
that history. Its startup margin advances the first settings write by at least 699 s from the
record's timestamp, instead of 2 s. **This is an estimate, not a guarantee:** it worked in the tested
restart scenarios, but Windows does not expose the highest timestamp `explorer.exe` has applied. The guide
explains the [margin and its cost](docs/color-temperature-transitions.md#one-handle-per-application-with-a-startup-margin).

The [timestamp rewind probe](probes/timestamp-rewind/README.md) measured these behaviors on Windows
11 build 26300:

- `explorer.exe` behaved as if it remembered the highest settings timestamp it had applied and
  ignored writes at or below it. Moving the stored timestamp backward can therefore stop display
  updates.
- Windows replaced settings writes below the separate `Cloud` timestamp with an earlier Windows
  settings value. `Cloud` usually followed Windows Settings writes; one `explorer.exe` startup write
  also updated it.
- After a forced `explorer.exe` restart, plain temperature writes were ignored until a slider interaction
  or a sufficiently long preview. About 9 ms worked in the measured trials; the library's brief
  preview uses 50 ms.

These are observations on one machine, not a documented Windows contract. The probe tested these
filtering and replacement behaviors for settings only, not state. See its README for the evidence,
exceptions, and instructions for reproducing the measurements.

### Types

| Type                           | Purpose                                                                                                                                  |
| :----------------------------- | :--------------------------------------------------------------------------------------------------------------------------------------- |
| `Nightlight`                   | Reads, writes, and updates the settings and state values.                                                                                |
| `Settings`                     | Schedule, color temperature, and preview; `from_bytes` and `to_bytes` round-trip every field except unknown fields inside a time of day. |
| `State`                        | Whether Night Light is on, the transition cause, and its time.                                                                           |
| `ColorTemperature`             | Validated color temperature from 1200K to 6500K.                                                                                         |
| `TimeOfDay`                    | 24-hour `HH:MM` time, with a check for whether it falls in a window that can wrap past midnight.                                         |
| `Schedule`                     | Schedule change passed to `Settings::set_schedule`.                                                                                      |
| `ScheduleMode`, `ScheduleKind` | Effective schedule mode, and the type kept while the schedule is off.                                                                    |
| `TransitionCause`              | Whether the last on/off change was manual or scheduled.                                                                                  |
| `Error`, `ErrorKind`, `Result` | Errors classified as not found, registry, decode, encode, invalid input, or preview cleanup.                                             |

### Registry format

Settings and state are stored as `REG_BINARY` values named `Data` under:

- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings`
- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate`

The format is Microsoft Bond CompactBinary v1 inside a CloudStore envelope. Microsoft does not
document it; the library follows reverse-engineered schemas. It preserves fields it does not model,
except unknown fields inside time-of-day records, which it discards during decoding.
See [`docs`](docs/) for the format details.

## `win-nightlight-watch`

Use the watcher to observe Night Light changes from a Rust application. Add it to your project:

```shell
cargo add win-nightlight-watch
```

Create `Watcher::settings()` or `Watcher::state()` for the record you need. Each constructor
registers for changes before it returns; call `read_current()` for its initial snapshot. If you need
both records, create both watchers before reading either.

`wait_timeout(Duration)` waits for a change, registers for the next notification, and reads the
current record through `win-nightlight-lib`. It returns `Some(Snapshot::Settings(...))` or
`Some(Snapshot::State(...))`, or `None` on timeout. The watcher does not start a thread.

A snapshot describes the value when it was read. Windows can combine notifications, and repeated
snapshots can describe the same data. The API does not capture the exact value that triggered a
notification or provide an atomic settings-and-state pair.

Use `AsHandle` or `AsRawHandle` with a native Windows wait or message loop, then call
`read_pending()` on the signaled watcher. An external wait does not consume the signal;
`read_pending()` consumes it. Each watcher is the only consumer of its notifications and registers
for the next notification itself.

If reads must be deferred, `discard_pending()` consumes the pending notification and registers for
the next one without reading. Call `read_current()` to retry the read when the application chooses.
After an error of kind `ErrorKind::Read`, the watcher stays registered for notifications. After a
notification failure, drop the watcher.

See [change notifications](docs/change-notifications.md) for lifecycle, timeout, and error
contracts, and the [crate documentation](https://docs.rs/win-nightlight-watch) for a compiled
usage example.
