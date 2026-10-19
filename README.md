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

Preview is a temporary tint that Windows shows while the user holds the Strength slider in Windows
Settings, even when Night Light is off. A program that exits in the middle of a change can leave
preview on, and the screen then stays tinted after Night Light turns off. `on`, `off`, `toggle`,
`temp`, and `schedule` also turn preview off. Turning preview off writes the settings value, and
only when preview was on.

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

Preview is a flag in the settings: while it is on, Windows shows the stored color temperature, even
while Night Light is off. Windows Settings turns it on while the user holds the Strength slider. A
ramp is a gradual change over more than 30 s, and a fade is a gradual change over up to 30 s.
`Nightlight` changes the temperature in three ways:

| Method                                                         | Writes                                                                                                                                                                  | Use                                   |
| :------------------------------------------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------ |
| `apply_color_temperature(t)`                                   | While Night Light is on: `t` with preview on, then preview off after 50 ms. While it is off: `t` with preview off, in one write. No write when both are already stored. | A one-off change; `wnl temp` calls it |
| `set_color_temperature(t)`                                     | `t` in one write, without changing preview. No write when `t` is already stored.                                                                                        | Storing a temperature, or a ramp step |
| `start_preview`, `preview_color_temperature(t)`, `end_preview` | One write each: preview on, then `t` with preview on, then preview off.                                                                                                 | A fade with preview held on           |

- `apply_color_temperature` ends any preview that was on and returns whether one was. While Night
  Light is on, it blocks the calling thread for the 50 ms wait. If turning preview off fails three
  times in a row, it returns an error of kind `ErrorKind::PreviewCleanup`, and preview may stay on;
  calling it again repeats the whole sequence.
- The preview methods each write once, even when nothing changes, and return whether preview was on
  before the call. They do not sleep.
- `update_settings` and `write_settings` write at most once and do not turn preview on or off
  themselves. `write_settings` stores the preview flag of the `Settings` it is given.

Each method in the table reads the settings and then writes them. Another writer, such as Windows
Settings or another call, can write in between, and the later write replaces the earlier one. Avoid
overlapping calls. A successful write does not confirm that the display changed.

### Settings timestamps

Each write of the settings or state value stores a last-modified timestamp in whole seconds.
Microsoft does not document how Windows uses it, and Windows behaves in ways a writer has to plan
for:

- `explorer.exe`, which applies Night Light to the display, ignores a settings write stamped at or
  below the highest timestamp it has applied. A program that moves the timestamp backward has its
  later writes ignored until one is stamped above that maximum.
- Windows keeps a third registry value, `Cloud`, next to the settings value. It stores only the
  timestamp of the latest Windows Settings write. When a program writes the settings stamped below
  that timestamp, Windows replaces the write with the last Windows Settings write.
- After `explorer.exe` restarts, it ignores temperature changes until a preview stays on for about 9
  ms or longer, or until the user moves the slider in Windows Settings. Switching Night Light on
  still shows the stored temperature. `apply_color_temperature` keeps preview on for 50 ms while
  Night Light is on; `set_color_temperature` does not turn it on.

The timestamp rewind probe, described below, measured these rules for the settings value only.

The library stamps each write 2 s after the stored timestamp, the same step Windows Settings uses,
or at the current time if that is later. Each `Nightlight` handle, the value that
`Nightlight::new()` returns, also stamps each write at least 2 s after the highest timestamp it has
written, so it never moves the timestamp below its own earlier writes. Writing more often than once
every 2 s therefore moves the timestamp ahead of real time, and the difference shrinks only as real
time passes. While the timestamp is ahead, Windows still applies each library write, because each is
stamped above the one before. The difference matters when another program writes the settings
stamped with the current time: Windows then ignores each write stamped at or below the highest
timestamp it applied before. A handle that wrote that timestamp stamps its next write above it, but
a new handle's writes are ignored until real time or the handle's 2-second steps pass it.

Keep one handle for the application's lifetime, and create it with
`Nightlight::new().with_startup_margin()`. That handle stamps its first settings write at least
74 s after the stored timestamp. During a fade of up to 240 writes per second, a slider drag in
Windows Settings can store a timestamp up to 74 s below the highest one Windows has applied, and
the registry does not store that highest timestamp.

The [timestamp rewind probe](probes/timestamp-rewind/README.md), a program in this repository,
measured these Windows behaviors on a live system. It writes Night Light in controlled sequences,
asks an observer at the display which tint the screen shows, and records registry access with
Process Monitor. Its cases cover moving the timestamp backward, writes below `Cloud`, `explorer.exe`
restarts, preview timings, fades made with preview held on, write rates, and writes without preview.
See the probe's README for the findings and how to rerun its cases.

For an application that changes the temperature gradually, the [color temperature transitions
guide](docs/color-temperature-transitions.md) prescribes which methods to call for one-off changes,
ramps, and fades and how often to write. It also describes what happens when the user moves the
slider during a gradual change.

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
