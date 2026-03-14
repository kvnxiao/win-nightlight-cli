# Windows Night Light Registry Format

Windows Night Light stores its configuration in the registry as binary blobs encoded with
[Bond CompactBinary v1](bond-compact-binary-v1.md). This document describes the registry
locations, the outer CloudStore wrapper structure, and the inner Night Light schemas.

## Background: CloudStore

Windows CloudStore (`Software\Microsoft\Windows\CurrentVersion\CloudStore\`) is an undocumented
local persistence layer for Windows settings sync. It stores shell personalization data (Start Menu
layout, Night Light, etc.) as Bond CompactBinary payloads. CloudStore data can be synced to
Microsoft's cloud via Windows Backup or Enterprise State Roaming.

There is no official Microsoft documentation for the CloudStore binary format.

## Registry Locations

Both values are `REG_BINARY` under `HKEY_CURRENT_USER`:

### Settings

```
Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\
  default$windows.data.bluelightreduction.settings\
  windows.data.bluelightreduction.settings
```
Value name: `Data`

Contains: schedule mode, color temperature, start/end times, sunset/sunrise times.

### State

```
Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\
  default$windows.data.bluelightreduction.bluelightreductionstate\
  windows.data.bluelightreduction.bluelightreductionstate
```
Value name: `Data`

Contains: whether Night Light is force-enabled (toggled on), and a FILETIME recording the last
state transition.

## Outer CloudStore Wrapper

Both Settings and State share the same outer structure — a marshaled Bond CompactBinary v1 struct:

```
Marshaled CB v1 header: [0x43, 0x42, 0x01, 0x00]

Field 0: BT_STRUCT                          // metadata
  Field 0: BT_BOOL = true                   // validity/presence flag (always true)
  BT_STOP

Field 1: BT_STRUCT                          // payload container
  Field 0: BT_UINT64 = <timestamp>          // last-modified Unix timestamp (seconds)
  Field 1: BT_STRUCT                        // data wrapper
    Field 1: BT_LIST<BT_INT8> = <payload>   // inner marshaled CB payload as byte blob
    BT_STOP
  BT_STOP

BT_STOP
```

The inner payload (carried as a `list<int8>`, effectively a byte blob) is itself another marshaled
CompactBinary v1 struct containing the actual Night Light data.

### Parsing the wrapper

1. Read the 4-byte marshaled header (`CB` magic + version 1).
2. Read Field 0 (struct with a bool) — metadata, always true.
3. Read Field 1 (struct):
   - Field 0 (uint64): the last-modified Unix timestamp in seconds.
   - Field 1 (struct) → Field 1 (list\<int8\>): extract the list elements as a contiguous byte
     array. This is the inner payload.
4. The inner struct's `BT_STOP` is the last element of the list. Three more `BT_STOP` bytes follow
   to close the three nested wrapper structs.

## Inner Settings Schema

The inner payload (after the CloudStore wrapper) is a marshaled CB v1 struct:

| Field ID | Bond Type | Name | Description |
|----------|-----------|------|-------------|
| 0 | `BT_BOOL` | `schedule_enabled` | `true` if any schedule mode is active |
| 10 | `BT_BOOL` | `set_hours_mode` | **Presence** indicates "Set Hours" mode. When absent and field 0 is true, mode is "Sunset to Sunrise". Value is always `false` (irrelevant — presence is the signal). |
| 20 | `BT_STRUCT` | `schedule_start_time` | TimeBlock: schedule start time |
| 30 | `BT_STRUCT` | `schedule_end_time` | TimeBlock: schedule end time |
| 40 | `BT_INT16` | `color_temperature` | Color temperature in Kelvin (1200–6500). Encoded via ZigZag + varint. |
| 50 | `BT_STRUCT` | `sunset_time` | TimeBlock: computed sunset time |
| 60 | `BT_STRUCT` | `sunrise_time` | TimeBlock: computed sunrise time |

### Schedule Mode Logic

| `schedule_enabled` (field 0) | `set_hours_mode` (field 10) | Mode |
|---|----|------|
| absent / false | — | Off |
| true | absent | Sunset to Sunrise |
| true | present | Set Hours |

### TimeBlock Sub-struct

Each time block is a struct with optional fields:

| Field ID | Bond Type | Name | Range | Default |
|----------|-----------|------|-------|---------|
| 0 | `BT_INT8` | `hour` | 0–23 | 0 |
| 1 | `BT_INT8` | `minute` | 0–59 | 0 |

When hour or minute is 0, the field is **omitted** (Bond default value omission). An empty struct
(immediate `BT_STOP`) represents midnight (00:00).

### Color Temperature Encoding

Field 40 is `BT_INT16`, which Bond encodes as ZigZag + varint:

```
encode: zigzag(2790) = 5580, varint(5580) = [0xCC, 0x2B]
decode: varint([0xCC, 0x2B]) = 5580, zigzag_decode(5580) = 2790
```

## Inner State Schema

| Field ID | Bond Type | Name | Description |
|----------|-----------|------|-------------|
| 0 | `BT_INT32` | `enabled_flag` | **Presence** = Night Light force-enabled (ON). When absent = OFF. Value is always 0 (irrelevant). |
| 10 | `BT_INT32` | `initialized` | Always 1. Likely a "data valid" or schema version marker. |
| 20 | `BT_UINT64` | `last_transition_filetime` | Windows FILETIME of the last state transition (toggle or scheduled change). |

### FILETIME Conversion

Field 20 is a [Windows FILETIME](https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-filetime) —
the number of 100-nanosecond intervals since January 1, 1601 (UTC).

```
unix_seconds = (filetime / 10_000_000) - 11_644_473_600
```

This timestamp records **when the Night Light state last changed** (e.g., a manual toggle or a
scheduled transition). It is distinct from the outer CloudStore Unix timestamp, which records when
the registry value was last written — these two events may differ.

### Enabled State Semantics

The presence of field 0 (not its value) determines enabled state:

| Field 0 present | Meaning |
|-----------------|---------|
| Yes (value=0) | Night Light is force-enabled (ON, regardless of schedule) |
| No | Night Light follows the schedule (or is OFF) |

## Annotated Byte Walkthrough

Settings example: schedule=SetHours, start=01:15, end=00:00, temp=2790K, sunset=19:23, sunrise=07:12.

```
-- Outer CloudStore wrapper --
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

    -- Inner Settings payload (38 bytes, itself a marshaled CB struct) --
    43 42 01 00         Marshaled header: CB v1
    02                  Field 0, BT_BOOL (schedule_enabled)
    01                    true
    C2 0A               Field 10, BT_BOOL (set_hours_mode)
    00                    false (value irrelevant; presence = set hours)
    CA 14               Field 20, BT_STRUCT (schedule_start_time)
      0E                  Field 0, BT_INT8 (hour)
      01                    1
      2E                  Field 1, BT_INT8 (minute)
      0F                    15
      00                  BT_STOP → time = 01:15
    CA 1E               Field 30, BT_STRUCT (schedule_end_time)
      00                  BT_STOP → time = 00:00 (fields omitted, defaults to 0)
    CF 28               Field 40, BT_INT16 (color_temperature)
    CC 2B                 zigzag varint = 2790 Kelvin
    CA 32               Field 50, BT_STRUCT (sunset_time)
      0E                  Field 0, BT_INT8 (hour)
      13                    19
      2E                  Field 1, BT_INT8 (minute)
      17                    23
      00                  BT_STOP → time = 19:23
    CA 3C               Field 60, BT_STRUCT (sunrise_time)
      0E                  Field 0, BT_INT8 (hour)
      07                    7
      2E                  Field 1, BT_INT8 (minute)
      0C                    12
      00                  BT_STOP → time = 07:12
    00                  BT_STOP (end inner settings struct; last list element)

  00                  BT_STOP (end data wrapper struct)
00                    BT_STOP (end payload container struct)
00                    BT_STOP (end outer struct)
```

Total: 60 bytes.

## References

- [Microsoft Bond](https://github.com/microsoft/bond) — the serialization framework
- [Bond CompactBinary v1 format](bond-compact-binary-v1.md) — wire format reference
- [Fleex's Lab: The Windows CloudStore](https://fleexlab.blogspot.com/2017/05/the-windows-cloudstore.html) — early reverse-engineering of CloudStore
- [Maclay74/tiny-screen NightLight.cs](https://github.com/Maclay74/tiny-screen) — C# Night Light implementation
- [nathanbabcock/nightlight-cli](https://github.com/nathanbabcock/nightlight-cli) — TypeScript port
- [fabsenet/adrilight NightlightDetection.md](https://github.com/fabsenet/adrilight/blob/main/NightlightDetection.md) — ML-based detection approach
- [Den Delimarsky: Parsing Halo API Bond data](https://den.dev/blog/parsing-halo-api-bond/) — Bond parsing techniques
