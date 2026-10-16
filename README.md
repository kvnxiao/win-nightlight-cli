# Windows Night Light libraries and CLI

Rust libraries and a CLI for Windows 11 Night Light:

- `win-nightlight-lib` reads and writes Night Light state, schedules, and color temperature.
- `win-nightlight-watch` waits for registry changes so applications can reread Night Light data.
- `win-nightlight-cli` installs the `wnl` command.

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

Add change notifications when needed:

```shell
cargo add win-nightlight-watch
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
schedule. `temp` sets the color temperature and preserves the schedule and on/off state. These
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

`Nightlight` is the entry point. Its temperature operation can read settings and state but writes
only settings. On/off operations write only state and preserve the schedule.

Preview is the temporary Night Light setting Windows Settings uses while adjusting its temperature
slider. `Nightlight::set_color_temperature` takes a `RefreshPolicy`. `Automatic`, used by `wnl temp`,
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

The writes run without a sleep. Each uses the later of the current Unix time and the timestamp in
the decoded `Settings` or `State` record plus two seconds, saturating at `u64::MAX`.
`write_settings` and `write_state` use the record supplied by the caller. Temperature updates read
settings first and reread them for cleanup. Rapid refreshes can advance timestamps ahead of the
clock.

Avoid overlapping temperature calls or using the Night Light slider in Windows Settings during a
call. Cleanup rereads
settings to preserve intervening changes, including a newer temperature, but registry updates are
not atomic with other writers. Successful writes do not confirm a display update or guarantee that
another writer has not changed the requested values.

If three cleanup attempts fail, `ErrorKind::PreviewCleanup` reports that the temperature was
written and preview may remain active. A later call with `Always` repeats the complete refresh,
including when the requested temperature is unchanged. `Automatic` preserves preexisting preview
and therefore does not provide that recovery contract.

`update_settings` and `write_settings` write at most once without adding this preview sequence.

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

Both values are stored as `REG_BINARY` values named `Data` under:

- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings`
- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate`

The format is Microsoft Bond CompactBinary v1 inside a CloudStore envelope. Microsoft does not
document it; the library follows reverse-engineered schemas and writes back unchanged the fields it
does not model, except fields inside a time of day. See [`docs`](docs/) for the format details.

### Migrate from 0.3 to 0.4

Add the required policy argument to `Nightlight::set_color_temperature`. Choose `Automatic` to
preserve 0.3 behavior, or `Always` when every call must request a complete preview refresh:

```rust
use win_nightlight_lib::RefreshPolicy;

nightlight.set_color_temperature(temperature, RefreshPolicy::Automatic)?;
nightlight.set_color_temperature(temperature, RefreshPolicy::Always)?;
```

The `Settings::set_color_temperature` setter and the `wnl temp` command keep their existing
interfaces. The CLI uses `Automatic`.

## `win-nightlight-watch`

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
