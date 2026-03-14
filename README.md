# `wnl` CLI / `win-nightlight-lib`

A Rust library and CLI for manipulating the Windows 11 night light settings.

**NOTE: Tested on Windows 11 24H2 (OS Build 26100.3476)**. This may not be guaranteed
to work on older Windows versions.

## `win-nightlight-lib`

The `win-nightlight-lib` library includes basic functionality to parse and modify the
Windows night light settings from the user's registry. The night light state and
settings are stored in a binary format located at:

- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate`
- `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings`

The binary format is Microsoft Bond CompactBinary v1, wrapped in a CloudStore
envelope. See [`docs`](docs/) for full format details.

## `wnl.exe` CLI Usage

```shell
Usage: wnl.exe <COMMAND>

Commands:
  temp      Sets the color temperature in Kelvin (1200 - 6500)
  schedule  Sets the schedule mode ('off', 'solar', or 'manual')
  on        Enables nightlight
  off       Disables nightlight
  status    Prints the current nightlight state and settings
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### `wnl schedule`

```shell
Usage: wnl.exe schedule [OPTIONS] <MODE>

Arguments:
  <MODE>  'off', 'solar', or 'manual'

Options:
  --start <HH:MM>  Start time for 'manual' mode (24-hour format)
  --end <HH:MM>    End time for 'manual' mode (24-hour format)

Examples:
  wnl schedule manual --start 22:00 --end 07:00
  wnl schedule manual --start 22:00
  wnl schedule solar
  wnl schedule off
```
