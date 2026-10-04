use super::BondError;

const MAX_VARINT_BYTES: usize = 10;

pub(super) fn read_varint(data: &[u8], pos: usize) -> Result<(u64, usize), BondError> {
    let mut value: u64 = 0;
    for (index, shift) in (0..MAX_VARINT_BYTES).zip((0..u64::BITS).step_by(7)) {
        let at = pos + index;
        let byte = *data.get(at).ok_or(BondError::UnexpectedEof(at))?;
        let part = u64::from(byte & 0x7F);
        if shift == 63 && part > 1 {
            return Err(BondError::VarintOverflow);
        }
        value |= part << shift;
        if byte < 0x80 {
            return Ok((value, at + 1));
        }
    }
    Err(BondError::VarintOverflow)
}

pub(super) fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let [low, ..] = value.to_le_bytes();
        let byte = low & 0x7F;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            return;
        }
        buf.push(byte | 0x80);
    }
}

pub(super) fn encode_zigzag_i16(value: i16) -> u16 {
    ((value << 1) ^ (value >> 15)).cast_unsigned()
}

pub(super) fn decode_zigzag_i16(value: u16) -> i16 {
    (value >> 1).cast_signed() ^ -(value & 1).cast_signed()
}

pub(super) fn encode_zigzag_i32(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)).cast_unsigned()
}

pub(super) fn decode_zigzag_i32(value: u32) -> i32 {
    (value >> 1).cast_signed() ^ -(value & 1).cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use test_case::test_case;

    #[test_case(0, &[0x00])]
    #[test_case(1, &[0x01])]
    #[test_case(127, &[0x7F])]
    #[test_case(128, &[0x80, 0x01])]
    #[test_case(300, &[0xAC, 0x02])]
    #[test_case(16_384, &[0x80, 0x80, 0x01])]
    #[test_case(1_742_540_908, &[0xEC, 0xA0, 0xF4, 0xBE, 0x06])]
    #[test_case(u64::MAX, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01])]
    fn varint_encoding(value: u64, encoded: &[u8]) {
        let mut buf = Vec::new();
        write_varint(&mut buf, value);
        assert_eq!(buf, encoded);
        assert_eq!(
            read_varint(encoded, 0).expect("encoding is valid"),
            (value, encoded.len())
        );
    }

    #[test_case(&[0x80]; "truncated")]
    #[test_case(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02]; "bit 64 set")]
    #[test_case(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x81, 0x00]; "eleven bytes")]
    fn varint_rejects(encoded: &[u8]) {
        let result = read_varint(encoded, 0);
        assert!(result.is_err(), "{result:?}");
    }

    #[test_case(0, 0)]
    #[test_case(-1, 1)]
    #[test_case(1, 2)]
    #[test_case(-2, 3)]
    #[test_case(2790, 5580)]
    #[test_case(i16::MAX, u16::MAX - 1)]
    #[test_case(i16::MIN, u16::MAX)]
    fn zigzag_i16(value: i16, encoded: u16) {
        assert_eq!(encode_zigzag_i16(value), encoded);
        assert_eq!(decode_zigzag_i16(encoded), value);
    }

    #[test_case(0, 0)]
    #[test_case(-1, 1)]
    #[test_case(1, 2)]
    #[test_case(i32::MAX, u32::MAX - 1)]
    #[test_case(i32::MIN, u32::MAX)]
    fn zigzag_i32(value: i32, encoded: u32) {
        assert_eq!(encode_zigzag_i32(value), encoded);
        assert_eq!(decode_zigzag_i32(encoded), value);
    }

    proptest! {
        #[test]
        fn varint_round_trip(value: u64) {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            prop_assert_eq!(read_varint(&buf, 0).expect("written varint decodes"), (value, buf.len()));
        }
    }
}
