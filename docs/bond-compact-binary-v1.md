# Bond CompactBinary v1 Wire Format

Microsoft [Bond](https://github.com/microsoft/bond) is a serialization framework. CompactBinary (CB) is
one of its binary protocols — a compact, self-describing wire format. This document describes the **v1**
encoding based on the reference implementation in
[`compact_binary.h`](https://github.com/microsoft/bond/blob/master/cpp/inc/bond/protocol/compact_binary.h)
and [`encoding.h`](https://github.com/microsoft/bond/blob/master/cpp/inc/bond/protocol/encoding.h).

## Marshaled Header

A marshaled CB payload begins with a 4-byte header:

| Offset | Size | Value | Description |
|--------|------|-------|-------------|
| 0 | 2 | `0x43 0x42` | Protocol magic (`COMPACT_PROTOCOL = 0x4243`, stored as uint16 LE) |
| 2 | 2 | `0x01 0x00` | Version 1 (uint16 LE) |

The magic bytes spell `"CB"` in ASCII.

## Type System

Bond types are identified by a 5-bit type ID in field headers:

| ID | Name | Value Encoding |
|----|------|----------------|
| 0 | `BT_STOP` | End of struct (no value) |
| 1 | `BT_STOP_BASE` | End of base class fields (no value) |
| 2 | `BT_BOOL` | 1 byte: `0x00` = false, `0x01` = true |
| 3 | `BT_UINT8` | 1 byte, raw |
| 4 | `BT_UINT16` | Unsigned varint |
| 5 | `BT_UINT32` | Unsigned varint |
| 6 | `BT_UINT64` | Unsigned varint |
| 7 | `BT_FLOAT` | 4 bytes, IEEE 754 little-endian |
| 8 | `BT_DOUBLE` | 8 bytes, IEEE 754 little-endian |
| 9 | `BT_STRING` | Varint byte-length + UTF-8 bytes |
| 10 | `BT_STRUCT` | Nested field sequence terminated by `BT_STOP` |
| 11 | `BT_LIST` | Container (see below) |
| 12 | `BT_SET` | Container (see below) |
| 13 | `BT_MAP` | Container (see below) |
| 14 | `BT_INT8` | 1 byte, raw (signed) |
| 15 | `BT_INT16` | ZigZag-encoded, then unsigned varint |
| 16 | `BT_INT32` | ZigZag-encoded, then unsigned varint |
| 17 | `BT_INT64` | ZigZag-encoded, then unsigned varint |
| 18 | `BT_WSTRING` | Varint code-unit-count + UTF-16LE bytes |

Source: [`bond_const.bond`](https://github.com/microsoft/bond/blob/master/idl/bond/core/bond_const.bond)

**Key detail:** `uint8` and `int8` are **raw single bytes**, NOT varint-encoded. All other integer
types (16/32/64-bit) use varint encoding.

## Field Headers

Each struct field is preceded by a header encoding the **absolute field ID** and the **type ID**.

### Encoding (1–3 bytes)

```
Byte 0:  [ id_bits (3) | type (5) ]
           bits 7-5       bits 4-0
```

The `id_bits` determine how the field ID is encoded:

| id_bits | Field ID range | Total header size | Layout |
|---------|----------------|-------------------|--------|
| 0–5 | 0–5 | 1 byte | `id_bits` IS the field ID |
| 6 (`0xC0`) | 6–255 | 2 bytes | Next byte = field ID (uint8) |
| 7 (`0xE0`) | 256–65535 | 3 bytes | Next 2 bytes = field ID (uint16 LE) |

Field IDs are **absolute** — there is no delta encoding between consecutive fields.

### Examples

| Field ID | Type | Header bytes |
|----------|------|--------------|
| 0 | `BT_BOOL` (2) | `0x02` |
| 1 | `BT_STRUCT` (10) | `0x2A` |
| 10 | `BT_BOOL` (2) | `0xC2 0x0A` |
| 40 | `BT_INT16` (15) | `0xCF 0x28` |
| 300 | `BT_UINT32` (5) | `0xE5 0x2C 0x01` |

### Struct Termination

- `BT_STOP` (`0x00`): ends the current struct.
- `BT_STOP_BASE` (`0x01`): ends base class fields in an inheritance hierarchy. Derived class
  fields follow, terminated by `BT_STOP`.

## Varint Encoding (LEB128)

Unsigned integers (except uint8) use Little-Endian Base-128 (LEB128) encoding:

- Each byte carries 7 data bits (bits 0–6) and 1 continuation bit (bit 7).
- Bit 7 = 1 means more bytes follow; bit 7 = 0 means this is the last byte.
- Bytes are ordered least-significant first.

### Write algorithm

```
fn write_varint(value: u64):
    loop:
        byte = (value & 0x7F) as u8
        value >>= 7
        if value != 0:
            byte |= 0x80
        emit(byte)
        if value == 0:
            break
```

### Read algorithm

```
fn read_varint() -> u64:
    result = 0
    shift = 0
    loop:
        byte = read_byte()
        result |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            break
    return result
```

### Examples

| Value | Encoded bytes | Explanation |
|-------|--------------|-------------|
| 0 | `00` | Single byte, no continuation |
| 127 | `7F` | Fits in 7 bits |
| 128 | `80 01` | Low 7 bits = 0, next 7 bits = 1 |
| 300 | `AC 02` | `300 = 0b100101100` → chunks: `0101100`, `10` |
| 16384 | `80 80 01` | Three bytes needed |

Reference: [`encoding.h:63–125`](https://github.com/microsoft/bond/blob/master/cpp/inc/bond/protocol/encoding.h#L63)

## ZigZag Encoding

Signed integers are first ZigZag-encoded into unsigned integers, then varint-encoded. ZigZag maps
signed values to unsigned values so that small-magnitude values (positive or negative) produce small
varints.

### Encode (signed → unsigned)

```
fn encode_zigzag(value: iN) -> uN:
    return (value << 1) ^ (value >> (N - 1))
```

### Decode (unsigned → signed)

```
fn decode_zigzag(value: uN) -> iN:
    return (value >> 1) ^ (-(value & 1))
```

### Mapping

| Signed | Unsigned |
|--------|----------|
| 0 | 0 |
| -1 | 1 |
| 1 | 2 |
| -2 | 3 |
| 2 | 4 |
| 2790 | 5580 |
| -2790 | 5579 |

Reference: [`encoding.h:140–154`](https://github.com/microsoft/bond/blob/master/cpp/inc/bond/protocol/encoding.h#L140)

## Container Encoding

### List and Set (v1)

```
[ element_type: u8 ] [ count: varint ] [ element_0 ] [ element_1 ] ... [ element_N-1 ]
```

- `element_type`: low 5 bits = BondType ID of elements (upper bits unused in v1).
- `count`: unsigned varint, number of elements.
- Elements are encoded sequentially using the element type's encoding.

> **Note:** CompactBinary v2 packs small counts (< 7) into the upper 3 bits of the type byte. v1
> does not use this optimization.

### Map

```
[ key_type: u8 ] [ value_type: u8 ] [ count: varint ] [ key_0 ] [ val_0 ] ... [ key_N-1 ] [ val_N-1 ]
```

- `key_type` and `value_type`: raw bytes, low 5 bits = BondType ID.
- `count`: unsigned varint, number of key-value pairs.
- Pairs are encoded as alternating key, value sequences.

Reference: [`compact_binary.h:273–299, 816–837`](https://github.com/microsoft/bond/blob/master/cpp/inc/bond/protocol/compact_binary.h#L273)

## Struct Encoding (v1)

A struct is a sequence of fields followed by `BT_STOP`:

```
[ field_header_0 ] [ field_value_0 ] [ field_header_1 ] [ field_value_1 ] ... [ BT_STOP ]
```

- Fields should be ordered by field ID (ascending).
- Fields with default values **may be omitted** — the writer simply skips them.
- Nested structs recursively follow this same format.
- In v1, there is **no length prefix** on structs (v2 adds a varint length before the fields).

### Inheritance

Structs with base classes encode base fields first, separated by `BT_STOP_BASE`:

```
[ base_field_0 ] ... [ BT_STOP_BASE ] [ derived_field_0 ] ... [ BT_STOP ]
```

## Default Value Omission

The CompactBinary protocol allows omitting fields that have their default value. When a field is absent
from the serialized data, the reader should use the field's default value. Standard defaults:

| Type | Default |
|------|---------|
| Bool | `false` |
| Integer types | `0` |
| Float / Double | `0.0` |
| String / WString | `""` (empty) |
| Containers | empty |
| Struct | default-constructed |

The writer decides whether to omit default-valued fields. The reader must handle absent fields gracefully.

## Full Example

A struct with 3 fields: field 0 = bool (true), field 10 = int32 (42), field 20 = struct { field 0 = uint64 (1000) }:

```
43 42 01 00       Marshaled header: CB v1
02                Field 0, BT_BOOL
01                  value = true
D0 0A             Field 10 (extended 1-byte ID), BT_INT32
54                  zigzag(42) = 84, varint(84) = 0x54
CA 14             Field 20 (extended 1-byte ID), BT_STRUCT
  06                Field 0, BT_UINT64
  E8 07             varint(1000) = [0xE8, 0x07]
  00                BT_STOP (end of nested struct)
00                BT_STOP (end of outer struct)
```
