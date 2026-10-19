# Windows Night Light Registry Format

Windows Night Light stores its configuration in two registry values encoded with
[Bond CompactBinary v1](bond-compact-binary-v1.md): a settings value for the schedule and color
temperature, and a state value for whether Night Light is on now. A third value, the `Cloud`
record, stores the timestamp of the latest Windows Settings write. This document describes the
registry locations, the outer CloudStore envelope, the inner Night Light schemas, and the rules a
writer follows so that Windows applies a write.

Microsoft does not document this format. The schemas below are reverse-engineered; the field names
and defaults come from [zomfg/NightLightLibrary](https://github.com/zomfg/NightLightLibrary)
(`nightlight_schema.bond`) and [Enyium/sem-reg-rs](https://github.com/Enyium/sem-reg-rs), and the
byte examples come from Windows 11 builds 26100 and 26300.

## Background: CloudStore

Windows CloudStore (`Software\Microsoft\Windows\CurrentVersion\CloudStore\`) is an undocumented
local persistence layer for Windows settings sync. It stores shell personalization data (Start Menu
layout, Night Light, and others) as Bond CompactBinary payloads. CloudStore data can sync to
Microsoft's cloud through Windows Backup or Enterprise State Roaming.

## Registry Locations

All three values are `REG_BINARY` values named `Data` under `HKEY_CURRENT_USER`.

### Settings

```
Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\
  default$windows.data.bluelightreduction.settings\
  windows.data.bluelightreduction.settings
```

Contains: whether the schedule is on, the schedule type, set-hours start and end times, the color
temperature, the computed sunset and sunrise times, and a preview flag.

### State

```
Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\
  default$windows.data.bluelightreduction.bluelightreductionstate\
  windows.data.bluelightreduction.bluelightreductionstate
```

Contains: whether Night Light is on now, what caused the last change, and when it happened.

### Cloud record

```
Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Cloud\
  default$windows.data.bluelightreduction.settings\
  windows.data.bluelightreduction.settings
```

Contains: only the timestamp of the latest settings write by Windows Settings. 44–62 ms after each
Windows Settings write, `explorer.exe` writes it with that write's timestamp. Settings writes by any
other program, such as one that uses this repository's `win-nightlight-lib` library, do not change
it. The record has no payload, and its layout differs from the envelope of the settings and state
values:

```
Marshaled CB v1 header: [0x43, 0x42, 0x01, 0x00]

Field 0: BT_STRUCT                          // metadata, empty
  BT_STOP

Field 1: BT_UINT64 = <timestamp>            // Unix seconds of the latest Windows Settings write

BT_STOP
```

For example, `43 42 01 00 0A 00 26 F5 90 D5 D6 06 00` records timestamp 1792362613.

## Outer CloudStore Envelope

The settings and state values share the same outer structure, a marshaled Bond CompactBinary v1
struct:

```
Marshaled CB v1 header: [0x43, 0x42, 0x01, 0x00]

Field 0: BT_STRUCT                          // metadata
  Field 0: BT_BOOL = true
  BT_STOP

Field 1: BT_STRUCT                          // payload container
  Field 0: BT_UINT64 = <timestamp>          // last-modified Unix timestamp (seconds)
  Field 1: BT_STRUCT                        // data wrapper
    Field 1: BT_LIST<BT_INT8> = <payload>   // inner marshaled CB payload as a byte blob
    BT_STOP
  BT_STOP

BT_STOP
```

The inner payload, stored as a `list<int8>`, is itself a marshaled CompactBinary v1 struct with
the Night Light fields. The inner struct's `BT_STOP` is the last byte of the list; three more
`BT_STOP` bytes close the data wrapper, the payload container, and the outer struct.

### Timestamp rule

The registry accepts a settings write with any envelope timestamp. Two Windows behaviors then depend
on the timestamp. The [timestamp rewind probe](../probes/timestamp-rewind/README.md), a program in
this repository that writes the settings and asks an observer what the screen shows, measured both
on build 26300:

- **Revert below `Cloud`:** a settings write stamped below the [`Cloud` record](#cloud-record)
  timestamp is replaced with the last Windows Settings write, including its timestamp and
  temperature. `explorer.exe` makes this revert, and so does `ShellHost.exe`, the process that shows
  Quick Settings, once Quick Settings has opened. Each usually reverts within 4 ms, and sometimes up
  to about 70 ms later. A revert overwrites any write made between the write below `Cloud` and that
  revert.
- **Stale-timestamp block after a rewind:** a rewind is a settings write that moves the timestamp
  backward but not below `Cloud`, so Windows keeps it. `explorer.exe`, which applies the settings to
  the display, behaves as if it keeps the highest timestamp it has applied. After a rewind, it
  ignores temperature changes stamped at or below that maximum; the probe calls this state a
  stale-timestamp block. The ignored changes include two kinds of temperature write: a write with
  preview (field 70) on followed at once by a write that turns preview off, and a write without
  preview. Without a block, both apply (findings 3 and 16 of the probe). A reverted write below
  `Cloud` clears the maximum (finding 4). An `explorer.exe` restart clears it too, but starts a
  different block, described in [Writer rules](#writer-rules) (finding 6).

Windows Settings stamps each of its writes two seconds after the last timestamp it read or wrote. A
writer that uses the same two-second step keeps the timestamp increasing: it sets the timestamp to
the later of the current Unix time and the previous timestamp plus two seconds. `win-nightlight-lib`
stamps the state value with the same rule; the probe measured the two behaviors above only for the
settings value.

```
modified = max(now_unix_seconds, previous_modified + 2)
```

Rapid writes can advance this whole-second timestamp ahead of the clock. The difference between two
such timestamps does not measure elapsed time. Reread the value before each subsequent write and
compute the next timestamp from that snapshot. The read and write are not atomic with other writers.

## Inner Settings Schema

| Field ID | Bond Type   | Default | Description                                                                                    |
| -------- | ----------- | ------- | ---------------------------------------------------------------------------------------------- |
| 0        | `BT_BOOL`   | `false` | `enabled`: `true` when the schedule is on. Omitted when `false`.                               |
| 10       | `BT_BOOL`   | `true`  | `onSunSchedule`: `true` for sunset to sunrise, `false` for set hours. Omitted when `true`.     |
| 20       | `BT_STRUCT` | 00:00   | TimeBlock: set-hours start time.                                                               |
| 30       | `BT_STRUCT` | 00:00   | TimeBlock: set-hours end time.                                                                 |
| 40       | `BT_INT16`  | 6500    | `colorTemperature`: color temperature in kelvin; the Settings app uses 1200 to 6500.           |
| 50       | `BT_STRUCT` | 00:00   | TimeBlock: sunset time Windows computes from the device location.                              |
| 60       | `BT_STRUCT` | 00:00   | TimeBlock: sunrise time Windows computes from the device location.                             |
| 70       | `BT_BOOL`   | `false` | `previewing`: `true` while Windows applies a temperature temporarily. Omitted when `false`.    |

Windows writes all four TimeBlocks, including a block for 00:00 (`CA 1E 00`). The sunset and
sunrise blocks stay at 00:00 until Windows computes them.

Field 40 can be absent, and Windows then applies its own default. zomfg/NightLightLibrary declares
the field `required`, but captured values in sem-reg-rs omit it.

### Schedule mode

The value of field 10, not its presence, selects the schedule type. Because its default is `true`,
an absent field 10 means sunset to sunrise. Windows keeps the type while the schedule is off.

| `enabled` (field 0) | `onSunSchedule` (field 10) | Mode              |
| ------------------- | -------------------------- | ----------------- |
| absent or `false`   | any                        | Off               |
| `true`              | absent or `true`           | Sunset to sunrise |
| `true`              | `false`                    | Set hours         |

### TimeBlock struct

| Field ID | Bond Type | Name     | Range | Default |
| -------- | --------- | -------- | ----- | ------- |
| 0        | `BT_INT8` | `hour`   | 0–23  | 0       |
| 1        | `BT_INT8` | `minute` | 0–59  | 0       |

Fields with the value 0 are omitted, so an empty struct (immediate `BT_STOP`) is 00:00.

### Color temperature encoding

Field 40 is `BT_INT16`, which Bond encodes as ZigZag plus varint:

```
encode: zigzag(2790) = 5580, varint(5580) = [0xCC, 0x2B]
decode: varint([0xCC, 0x2B]) = 5580, zigzag_decode(5580) = 2790
```

### Observed native slider behavior

Windows 11 Settings uses field 70 to preview the selected color temperature. During a drag,
preview remains true across temperature updates and returns to false afterward. The native UI
also previews the selected temperature while Night Light is off.

Clicking the slider to jump to a new strength produced three successive settings values:

1. The new temperature, with preview false.
2. The same temperature, with preview true.
3. The same temperature, with preview false.

In the captured settings values, only fields 40 and 70 changed. Each write also advanced the
envelope timestamp. These are observations of the native UI, not a documented Windows API
contract.

A capture on 2026-10-16 UTC recorded slider drags and four click interactions; the table below lists
measurements of the clicks. The recording program used one `Watcher` per key from this repository's
`win-nightlight-watch` crate. A `Watcher` registers for notifications through
`RegNotifyChangeKeyValue`. The program sampled its clocks after `WaitForMultipleObjects` returned a
notification and before registering for the next notification and reading the value. Each interval
below runs from the snapshot with preview true to the next snapshot with preview false:

| Click | Notification observation interval | Registry-key last-write interval | Envelope timestamp increase |
| ----- | --------------------------------- | -------------------------------- | --------------------------- |
| 1     | 60.096 ms                         | 60.173 ms                        | 2 seconds                   |
| 2     | 72.958 ms                         | 72.323 ms                        | 2 seconds                   |
| 3     | 51.225 ms                         | 51.204 ms                        | 2 seconds                   |
| 4     | 96.816 ms                         | 96.986 ms                        | 2 seconds                   |

The recording used `QueryPerformanceCounter` at 10 MHz and `GetSystemTimePreciseAsFileTime`
around each registry read. The registry-key times came from `RegQueryInfoKeyW`. Clock samples
bracketing the reads were 2.2–10.8 microseconds apart. Notifications were observed 0.093–1.532 ms
after the corresponding key's last-write time; the median was 0.706 ms.

Across the drag and click interactions, 59 consecutive settings timestamp increments were exactly
two seconds. The timestamp advanced 118 seconds during 6.230 seconds of observed activity. The
UTC sample after the final settings read was `2026-10-16T02:03:29.2943260Z`, while the raw envelope
value was `1792116321`, or `2026-10-16T02:05:21Z`: 111.705674 seconds ahead of that clock sample.

These measurements describe notification observations and registry metadata, not exact write-call
or display-update times. Notifications can combine writes, and the value and metadata reads are
not atomic. The measured preview intervals do not establish a required minimum delay or verify
that back-to-back preview writes update the display.

## Inner State Schema

| Field ID | Bond Type   | Default | Description                                                                              |
| -------- | ----------- | ------- | ---------------------------------------------------------------------------------------- |
| 0        | `BT_INT32`  | absent  | Status: present when Night Light is on now, with the value 0 (running). Absent when off. |
| 10       | `BT_INT32`  | 0       | Transition cause: 1 for manual, 0 for scheduled. Omitted when 0.                         |
| 20       | `BT_UINT64` | 0       | FILETIME of the last change between on and off.                                          |
| 30       | `BT_BOOL`   | `true`  | `usable`: whether Night Light is usable on this device. Omitted when `true`.             |

Windows also writes the state value when the Night Light schedule turns Night Light on or off: it
sets or clears field 0 and records a scheduled cause. Field 0 therefore means "on now", not
"forced on". A manual change sets field 10 to 1 and field 20 to the current time.

### FILETIME conversion

Field 20 is a [Windows FILETIME](https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-filetime):
the number of 100-nanosecond intervals since January 1, 1601 (UTC).

```
unix_seconds = (filetime / 10_000_000) - 11_644_473_600
```

This field records when Night Light last turned on or off. The envelope timestamp advances on each
write and can be ahead of the clock; it does not give the precise time of a change.

## Writer rules

A writer that changes one value does not modify the other value:

- Turning Night Light on or off writes only the state value. It sets or clears field 0, sets
  field 10 to 1, sets field 20 to the current FILETIME, and stamps the envelope timestamp.
- Changing the schedule or the color temperature writes only the settings value and stamps the
  envelope timestamp.

`win-nightlight-lib`, the Rust library in this repository, changes the temperature in three ways,
and each writes only settings:

- `Nightlight::set_color_temperature` writes the temperature once without changing field 70, and
  does not write when the temperature already matches.
- `Nightlight::apply_color_temperature` reads the settings and the on/off state before writing:
  - While Night Light is on, it writes the temperature with field 70 true, waits 50 ms, then rereads
    the settings and, if field 70 is still true, writes them with field 70 false. A click on the
    native slider keeps preview on for a similar 51 to 97 ms, though it also writes the temperature
    with preview off first. Because the second write starts from a fresh read, it keeps a newer
    temperature that another program wrote during the wait.
  - While Night Light is off, it writes the temperature with field 70 false in one write, so the
    screen stays untinted, and does not write when both already match. The native slider instead
    previews the temperature while Night Light is off.
- `Nightlight::start_preview`, `Nightlight::preview_color_temperature`, and
  `Nightlight::end_preview` write the settings once per call, as a drag of the native slider does:
  preview on with the temperature unchanged, a temperature with preview on, and preview off. Preview
  stays on from `start_preview` to `end_preview`, so the screen shows each temperature, even while
  Night Light is off.

After `explorer.exe` is ended and restarted, Windows ignores both writes without preview and writes
that turn preview on and off at once. Writes apply as before once the user moves the native slider
or a preview stays on for about 9 ms or longer. `apply_color_temperature` keeps preview on that long
while Night Light is on, and so do `preview_color_temperature` calls between `start_preview` and
`end_preview`; `set_color_temperature` does not. See finding 7 of the [timestamp rewind
probe](../probes/timestamp-rewind/README.md#7-every-explorerexe-restart-blocks-ordinary-refreshes).
The [color temperature transitions guide](color-temperature-transitions.md) describes when to use
each method.

`Nightlight::update_settings` reads the settings, applies a caller's change, and writes them only
when the change modified them. `Nightlight::write_settings` writes a `Settings` record the caller
supplies. Both write at most once, do not turn field 70 on or off themselves, and store the field
70 of the record they write.

A writer also preserves fields it does not model, at the inner struct and at every envelope level,
and writes them back in ascending field-ID order. `win-nightlight-lib` does not preserve unknown
fields inside a TimeBlock.

## Annotated Byte Walkthrough

Settings example: schedule on, set hours, start 01:15, end 00:00, 2790K, sunset 19:23, sunrise
07:12.

```
-- Outer CloudStore envelope --
43 42 01 00           Marshaled header: CB v1 (magic 0x4243 + version 1)
0A                    Field 0, BT_STRUCT (metadata)
  02                    Field 0, BT_BOOL
  01                      true
  00                    BT_STOP
2A                    Field 1, BT_STRUCT (payload container)
  06                    Field 0, BT_UINT64 (timestamp)
  EC A0 F4 BE 06        varint = 1742540908 (Unix seconds)
  2A                    Field 1, BT_STRUCT (data wrapper)
    2B                    Field 1, BT_LIST
    0E                      element type = BT_INT8 (14)
    26                      count = 38 (38 bytes of inner payload)

    -- Inner settings payload (38 bytes, a marshaled CB struct) --
    43 42 01 00         Marshaled header: CB v1
    02                  Field 0, BT_BOOL (enabled)
    01                    true
    C2 0A               Field 10, BT_BOOL (onSunSchedule)
    00                    false → set hours
    CA 14               Field 20, BT_STRUCT (set-hours start)
      0E                  Field 0, BT_INT8 (hour)
      01                    1
      2E                  Field 1, BT_INT8 (minute)
      0F                    15
      00                  BT_STOP → 01:15
    CA 1E               Field 30, BT_STRUCT (set-hours end)
      00                  BT_STOP → 00:00 (fields omitted, both 0)
    CF 28               Field 40, BT_INT16 (colorTemperature)
    CC 2B                 zigzag varint = 2790 kelvin
    CA 32               Field 50, BT_STRUCT (sunset)
      0E                  Field 0, BT_INT8 (hour)
      13                    19
      2E                  Field 1, BT_INT8 (minute)
      17                    23
      00                  BT_STOP → 19:23
    CA 3C               Field 60, BT_STRUCT (sunrise)
      0E                  Field 0, BT_INT8 (hour)
      07                    7
      2E                  Field 1, BT_INT8 (minute)
      0C                    12
      00                  BT_STOP → 07:12
    00                  BT_STOP (end of inner settings struct; last list element)

  00                  BT_STOP (end of data wrapper)
00                    BT_STOP (end of payload container)
00                    BT_STOP (end of outer struct)
```

Total: 60 bytes.

State example: off, last changed by hand at 2026-03-26 09:04:27 UTC (inner payload only).

```
43 42 01 00           Marshaled header: CB v1
D0 0A                 Field 10, BT_INT32 (transition cause)
02                      zigzag varint = 1 → manual
C6 14                 Field 20, BT_UINT64 (last transition)
B0 9F D1 E6 F8 9F AF EE 01
                        varint = 134189894679678896 (FILETIME)
00                    BT_STOP; field 0 is absent → off
```

## References

- [zomfg/NightLightLibrary](https://github.com/zomfg/NightLightLibrary) — reverse-engineered Bond schema (`nightlight_schema.bond`) with field names and defaults
- [Enyium/sem-reg-rs](https://github.com/Enyium/sem-reg-rs) — Rust implementation of the Night Light settings and state values
- [Microsoft Bond](https://github.com/microsoft/bond) — the serialization framework
- [Bond CompactBinary v1 format](bond-compact-binary-v1.md) — wire format reference
- [Fleex's Lab: The Windows CloudStore](https://fleexlab.blogspot.com/2017/05/the-windows-cloudstore.html) — early reverse engineering of CloudStore
- [Maclay74/tiny-screen NightLight.cs](https://github.com/Maclay74/tiny-screen) — C# Night Light implementation
- [nathanbabcock/nightlight-cli](https://github.com/nathanbabcock/nightlight-cli) — TypeScript port
- [fabsenet/adrilight NightlightDetection.md](https://github.com/fabsenet/adrilight/blob/main/NightlightDetection.md) — ML-based detection approach
- [Den Delimarsky: Parsing Halo API Bond data](https://den.dev/blog/parsing-halo-api-bond/) — Bond parsing techniques
