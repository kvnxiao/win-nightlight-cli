# Windows Night Light Registry Format

Windows Night Light stores its configuration in two registry values encoded with
[Bond CompactBinary v1](bond-compact-binary-v1.md): a settings value for the schedule and color
temperature, and a state value for whether Night Light is on now. This document describes the
registry locations, the outer CloudStore envelope, the inner Night Light schemas, and the rules a
writer follows so that Windows accepts a write.

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

Both values are `REG_BINARY` values named `Data` under `HKEY_CURRENT_USER`.

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

## Outer CloudStore Envelope

Both values share the same outer structure, a marshaled Bond CompactBinary v1 struct:

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

The inner payload, carried as a `list<int8>`, is itself a marshaled CompactBinary v1 struct with
the Night Light fields. The inner struct's `BT_STOP` is the last byte of the list; three more
`BT_STOP` bytes close the data wrapper, the payload container, and the outer struct.

### Timestamp rule

Windows discards a write whose envelope timestamp is less than the stored timestamp, and advances
the timestamp by at least two seconds on its own writes. A writer therefore sets the timestamp to
the later of the current Unix time and the previous timestamp plus two seconds:

```
modified = max(now_unix_seconds, previous_modified + 2)
```

## Inner Settings Schema

| Field ID | Bond Type   | Default | Description                                                                                    |
| -------- | ----------- | ------- | ---------------------------------------------------------------------------------------------- |
| 0        | `BT_BOOL`   | `false` | `enabled`: `true` when the schedule is on. Omitted when `false`.                               |
| 10       | `BT_BOOL`   | `true`  | `onSunSchedule`: `true` for sunset to sunrise, `false` for set hours. Written only as `false`. |
| 20       | `BT_STRUCT` | 00:00   | TimeBlock: set-hours start time.                                                               |
| 30       | `BT_STRUCT` | 00:00   | TimeBlock: set-hours end time.                                                                 |
| 40       | `BT_INT16`  | 6500    | `colorTemperature`: color temperature in kelvin; the Settings app uses 1200 to 6500. |
| 50       | `BT_STRUCT` | 00:00   | TimeBlock: sunset time Windows computes from the device location.                              |
| 60       | `BT_STRUCT` | 00:00   | TimeBlock: sunrise time Windows computes from the device location.                             |
| 70       | `BT_BOOL`   | `false` | `previewing`: `true` while the user drags the color temperature slider. Omitted when `false`.  |

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

## Inner State Schema

| Field ID | Bond Type   | Default | Description                                                                              |
| -------- | ----------- | ------- | ---------------------------------------------------------------------------------------- |
| 0        | `BT_INT32`  | absent  | Status: present when Night Light is on now, with the value 0 (running). Absent when off. |
| 10       | `BT_INT32`  | 0       | Transition cause: 1 for manual, 0 for scheduled. Omitted when 0.                         |
| 20       | `BT_UINT64` | 0       | FILETIME of the last change between on and off.                                          |
| 30       | `BT_BOOL`   | `true`  | `usable`: whether Night Light is usable on this device. Written only as `false`.         |

The Windows scheduler writes the state value too: at a scheduled transition it sets or clears
field 0 and records a scheduled cause. Field 0 therefore means "on now", not "forced on". A manual
change sets field 10 to 1 and field 20 to the current time.

### FILETIME conversion

Field 20 is a [Windows FILETIME](https://learn.microsoft.com/en-us/windows/win32/api/minwinbase/ns-minwinbase-filetime):
the number of 100-nanosecond intervals since January 1, 1601 (UTC).

```
unix_seconds = (filetime / 10_000_000) - 11_644_473_600
```

This field records when Night Light last turned on or off. The envelope timestamp records when the
registry value was last written; the two can differ.

## Writer rules

A writer that changes one value does not modify the other value:

- Turning Night Light on or off writes only the state value. It sets or clears field 0, sets
  field 10 to 1, sets field 20 to the current FILETIME, and stamps the envelope timestamp.
- Changing the schedule or the color temperature writes only the settings value and stamps the
  envelope timestamp.

A writer also preserves fields it does not model, at the inner struct and at every envelope level,
and writes them back in ascending field-ID order. This library does not preserve unknown fields
inside a TimeBlock.

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
