use super::BondError;

/// Reads an unsigned varint (LEB128) from `data` starting at `pos`.
/// Returns the decoded value and the new position after the varint.
pub(super) fn read_varint(data: &[u8], pos: usize) -> Result<(u64, usize), BondError> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    let mut pos = pos;

    loop {
        let byte = *data.get(pos).ok_or(BondError::UnexpectedEof(pos))?;
        pos += 1;

        let part = u64::from(byte & 0x7F);
        value |= part.checked_shl(shift).ok_or(BondError::VarintOverflow)?;
        shift += 7;

        if byte < 0x80 {
            break;
        }
    }

    Ok((value, pos))
}

/// Writes an unsigned varint (LEB128) into `buf`.
pub(super) fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let [low, ..] = value.to_le_bytes();
        let mut byte = low & 0x7F;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// ZigZag-encodes a signed 16-bit integer to an unsigned 16-bit integer.
pub(super) fn encode_zigzag_i16(value: i16) -> u16 {
    ((value << 1) ^ (value >> 15)).cast_unsigned()
}

/// ZigZag-decodes an unsigned 16-bit integer to a signed 16-bit integer.
pub(super) fn decode_zigzag_i16(value: u16) -> i16 {
    (value >> 1).cast_signed() ^ -(value & 1).cast_signed()
}

/// ZigZag-encodes a signed 32-bit integer to an unsigned 32-bit integer.
pub(super) fn encode_zigzag_i32(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)).cast_unsigned()
}

/// ZigZag-decodes an unsigned 32-bit integer to a signed 32-bit integer.
pub(super) fn decode_zigzag_i32(value: u32) -> i32 {
    (value >> 1).cast_signed() ^ -(value & 1).cast_signed()
}

/// ZigZag-encodes a signed 64-bit integer to an unsigned 64-bit integer.
#[cfg(test)]
pub(super) fn encode_zigzag_i64(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)).cast_unsigned()
}

/// ZigZag-decodes an unsigned 64-bit integer to a signed 64-bit integer.
#[cfg(test)]
pub(super) fn decode_zigzag_i64(value: u64) -> i64 {
    (value >> 1).cast_signed() ^ -(value & 1).cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip() {
        let cases: &[u64] = &[
            0,
            1,
            127,
            128,
            255,
            256,
            300,
            16383,
            16384,
            u64::from(u32::MAX),
            u64::MAX,
        ];
        for &val in cases {
            let mut buf = Vec::new();
            write_varint(&mut buf, val);
            let (decoded, end_pos) = read_varint(&buf, 0).expect("varint decodes");
            assert_eq!(val, decoded, "varint roundtrip failed for {val}");
            assert_eq!(end_pos, buf.len());
        }
    }

    #[test]
    fn varint_known_encodings() {
        // 300 = 0b100101100 → [0xAC, 0x02]
        let mut buf = Vec::new();
        write_varint(&mut buf, 300);
        assert_eq!(buf, [0xAC, 0x02]);

        // 128 → [0x80, 0x01]
        buf.clear();
        write_varint(&mut buf, 128);
        assert_eq!(buf, [0x80, 0x01]);

        // 0 → [0x00]
        buf.clear();
        write_varint(&mut buf, 0);
        assert_eq!(buf, [0x00]);
    }

    #[test]
    fn varint_timestamp() {
        // The Night Light timestamp 1742540908
        let mut buf = Vec::new();
        write_varint(&mut buf, 1_742_540_908);
        assert_eq!(buf, [0xEC, 0xA0, 0xF4, 0xBE, 0x06]);
        let (decoded, _) = read_varint(&buf, 0).expect("varint decodes");
        assert_eq!(decoded, 1_742_540_908);
    }

    #[test]
    fn zigzag_i16_roundtrip() {
        let cases: &[i16] = &[0, -1, 1, -2, 2, 2790, -2790, i16::MIN, i16::MAX];
        for &val in cases {
            let encoded = encode_zigzag_i16(val);
            let decoded = decode_zigzag_i16(encoded);
            assert_eq!(val, decoded, "zigzag i16 roundtrip failed for {val}");
        }
    }

    #[test]
    fn zigzag_i32_roundtrip() {
        let cases: &[i32] = &[0, -1, 1, -2, 2, 42, -42, i32::MIN, i32::MAX];
        for &val in cases {
            let encoded = encode_zigzag_i32(val);
            let decoded = decode_zigzag_i32(encoded);
            assert_eq!(val, decoded, "zigzag i32 roundtrip failed for {val}");
        }
    }

    #[test]
    fn zigzag_i64_roundtrip() {
        let cases: &[i64] = &[0, -1, 1, -2, 2, i64::MIN, i64::MAX];
        for &val in cases {
            let encoded = encode_zigzag_i64(val);
            let decoded = decode_zigzag_i64(encoded);
            assert_eq!(val, decoded, "zigzag i64 roundtrip failed for {val}");
        }
    }

    #[test]
    fn zigzag_known_values() {
        assert_eq!(encode_zigzag_i32(0), 0);
        assert_eq!(encode_zigzag_i32(-1), 1);
        assert_eq!(encode_zigzag_i32(1), 2);
        assert_eq!(encode_zigzag_i32(-2), 3);
        assert_eq!(encode_zigzag_i32(2), 4);

        // 2790 Kelvin: zigzag = 5580
        assert_eq!(encode_zigzag_i16(2790), 5580);
    }

    #[test]
    fn zigzag_varint_color_temperature() {
        // 2790K → zigzag(2790) = 5580 → varint(5580) = [0xCC, 0x2B]
        let zigzag = encode_zigzag_i16(2790);
        assert_eq!(zigzag, 5580);
        let mut buf = Vec::new();
        write_varint(&mut buf, u64::from(zigzag));
        assert_eq!(buf, [0xCC, 0x2B]);

        // Decode
        let (raw, _) = read_varint(&buf, 0).expect("varint decodes");
        let decoded = decode_zigzag_i16(u16::try_from(raw).expect("varint fits in u16"));
        assert_eq!(decoded, 2790);
    }
}
