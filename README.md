# `wnl` CLI / `win-nightlight-lib`

A Rust library and CLI that turn Windows 11 Night Light on and off and configure its schedule and
color temperature.

**NOTE: Tested on Windows 11 24H2 (OS build 26100) and build 26300.** Older Windows versions may
store Night Light differently.

## Installation

Install the `wnl` CLI from crates.io:

```shell
cargo install --locked win-nightlight-cli
```

Add the library to a project:

```shell
cargo add win-nightlight-lib
```

## `wnl` CLI usage

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

`on`, `off`, and `toggle` change only whether Night Light is on now; they do not change the
schedule. `temp` changes only the color temperature and does not turn Night Light on. These
commands print nothing on success.

```shell
wnl on
wnl off
wnl toggle
wnl temp 3400
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

The library reads and writes the two registry values that store Night Light:

- `Settings`: the schedule mode and times, the color temperature, and the sunset and sunrise times
  Windows computes.
- `State`: whether Night Light is on now, what caused the last change, and when it happened.

`Nightlight` is the entry point. It reads and writes each value on its own, so turning Night Light
on or off never changes the schedule, and changing a setting never turns Night Light on or off.
Each write stamps the CloudStore timestamp so Windows accepts it.

```rust
use win_nightlight_lib::ColorTemperature;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::Schedule;

let nightlight = Nightlight::new();
nightlight.set_active(true)?;

let temperature: ColorTemperature = "3400K".parse()?;
nightlight.update_settings(|settings| {
    settings.set_color_temperature(temperature);
    settings.set_schedule(Schedule::SunsetToSunrise);
})?;
```

| Type                           | Purpose                                                                 |
| :----------------------------- | :---------------------------------------------------------------------- |
| `Nightlight`                   | Reads, writes, and updates the settings and state values.               |
| `Settings`                     | Schedule and color temperature; `from_bytes` and `to_bytes` round-trip. |
| `State`                        | Whether Night Light is on, the transition cause, and its time.          |
| `ColorTemperature`             | Validated color temperature from 1200K to 6500K.                        |
| `TimeOfDay`                    | 24-hour `HH:MM` time with a midnight-wrapping window check.             |
| `Schedule`                     | Schedule change passed to `Settings::set_schedule`.                     |
| `ScheduleMode`, `ScheduleKind` | Effective schedule mode, and the type kept while the schedule is off.   |
| `TransitionCause`              | Whether the last on/off change was manual or scheduled.                 |
| `Error`, `ErrorKind`, `Result` | Errors classified as not found, registry, decode, encode, or input.     |

Both values are stored as `REG_BINARY` values named `Data` under:

- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings`
- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate`

The format is Microsoft Bond CompactBinary v1 inside a CloudStore envelope. Microsoft does not
document it; the library follows reverse-engineered schemas and writes back unchanged the fields it
does not model, except fields inside a time of day. See [`docs`](docs/) for the format details.
