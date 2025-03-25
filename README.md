# `wnl` CLI / `win-nightlight-lib`

A Rust library and CLI for manipulating the Windows night light settings.

## `win-nightlight-lib`

The `win-nightlight-lib` library includes basic functionality to parse the Windows
night light settings from the user's registry. The night light state and settings are
stored in a binary format located at:

- `Computer\HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings`
- `Computer\HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate`

As the format is in binary, the implementation for serialization & deserialization is
done at a best-effort basis based on resources found online for format interpretations.
See the documentation for `NightlightSettings` and `NightlightState` for more details
regarding the data structure interpretations. **Currently, the only unknown part about
the data structures is related to the latter bytes of the `NightlightState` - any
contribution towards this would be greatly appreciated!**

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
