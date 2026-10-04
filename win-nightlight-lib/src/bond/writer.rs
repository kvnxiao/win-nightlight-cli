use super::types::BondType;
use super::types::COMPACT_BINARY_MAGIC;
use super::types::COMPACT_BINARY_V1;
use super::unknown::UnknownField;
use super::unknown::UnknownFields;
use super::varint::encode_zigzag_i16;
use super::varint::encode_zigzag_i32;
use super::varint::write_varint;
use std::iter::Peekable;
use std::slice;

#[derive(Debug, Default)]
pub(crate) struct CompactBinaryWriter {
    buf: Vec<u8>,
}

impl CompactBinaryWriter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub(crate) fn write_marshaled_header(&mut self) {
        self.buf.extend_from_slice(&COMPACT_BINARY_MAGIC);
        self.buf.extend_from_slice(&COMPACT_BINARY_V1);
    }

    /// Write one struct whose known fields `fields` writes in ascending ID
    /// order, merging `unknown` fields into that order, then write the
    /// terminator.
    pub(crate) fn write_struct(
        &mut self,
        unknown: &UnknownFields,
        fields: impl FnOnce(&mut StructWriter<'_, '_>),
    ) {
        let mut writer = StructWriter {
            writer: self,
            pending: unknown.iter().peekable(),
        };
        fields(&mut writer);
        writer.flush_before(None);
        writer.writer.buf.push(0x00);
    }

    pub(crate) fn write_bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    pub(crate) fn write_int8(&mut self, value: i8) {
        self.buf.push(value.cast_unsigned());
    }

    pub(crate) fn write_int16(&mut self, value: i16) {
        write_varint(&mut self.buf, u64::from(encode_zigzag_i16(value)));
    }

    pub(crate) fn write_int32(&mut self, value: i32) {
        write_varint(&mut self.buf, u64::from(encode_zigzag_i32(value)));
    }

    pub(crate) fn write_uint64(&mut self, value: u64) {
        write_varint(&mut self.buf, value);
    }

    pub(crate) fn write_list_header(&mut self, element_type: BondType, count: u32) {
        self.buf.push(element_type.id());
        write_varint(&mut self.buf, u64::from(count));
    }

    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    fn write_field_header(&mut self, id: u16, bond_type: BondType) {
        let type_id = bond_type.id();
        let [low, high] = id.to_le_bytes();
        match id {
            0..=5 => self.buf.push(type_id | (low << 5)),
            6..=0xFF => self.buf.extend_from_slice(&[type_id | (6 << 5), low]),
            _ => self.buf.extend_from_slice(&[type_id | (7 << 5), low, high]),
        }
    }
}

pub(crate) struct StructWriter<'w, 'u> {
    writer: &'w mut CompactBinaryWriter,
    pending: Peekable<slice::Iter<'u, UnknownField>>,
}

impl StructWriter<'_, '_> {
    pub(crate) fn field(
        &mut self,
        id: u16,
        bond_type: BondType,
        value: impl FnOnce(&mut CompactBinaryWriter),
    ) {
        self.flush_before(Some(id));
        self.writer.write_field_header(id, bond_type);
        value(self.writer);
    }

    pub(crate) fn bool(&mut self, id: u16, value: bool) {
        self.field(id, BondType::Bool, |w| w.write_bool(value));
    }

    pub(crate) fn int8(&mut self, id: u16, value: i8) {
        self.field(id, BondType::Int8, |w| w.write_int8(value));
    }

    pub(crate) fn int16(&mut self, id: u16, value: i16) {
        self.field(id, BondType::Int16, |w| w.write_int16(value));
    }

    pub(crate) fn int32(&mut self, id: u16, value: i32) {
        self.field(id, BondType::Int32, |w| w.write_int32(value));
    }

    pub(crate) fn uint64(&mut self, id: u16, value: u64) {
        self.field(id, BondType::UInt64, |w| w.write_uint64(value));
    }

    fn flush_before(&mut self, id: Option<u16>) {
        while let Some(field) = self
            .pending
            .next_if(|field| id.is_none_or(|id| field.id < id))
        {
            self.writer.write_field_header(field.id, field.bond_type);
            self.writer.write_bytes(&field.value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bond::CompactBinaryReader;
    use crate::bond::expect_type;
    use test_case::test_case;

    #[test_case(0, BondType::Bool, &[0x02])]
    #[test_case(1, BondType::Struct, &[0x2A])]
    #[test_case(5, BondType::UInt64, &[0xA6])]
    #[test_case(6, BondType::Int8, &[0xCE, 0x06])]
    #[test_case(10, BondType::Bool, &[0xC2, 0x0A])]
    #[test_case(40, BondType::Int16, &[0xCF, 0x28])]
    #[test_case(255, BondType::Int32, &[0xD0, 0xFF])]
    #[test_case(256, BondType::Int32, &[0xF0, 0x00, 0x01])]
    #[test_case(300, BondType::UInt32, &[0xE5, 0x2C, 0x01])]
    fn writes_field_header(id: u16, bond_type: BondType, expected: &[u8]) {
        let mut writer = CompactBinaryWriter::new();
        writer.write_field_header(id, bond_type);
        assert_eq!(writer.into_bytes(), expected);
    }

    #[test]
    fn writes_marshaled_header() {
        let mut writer = CompactBinaryWriter::new();
        writer.write_marshaled_header();
        assert_eq!(writer.into_bytes(), [0x43, 0x42, 0x01, 0x00]);
    }

    #[test]
    fn writes_scalar_values() {
        let mut writer = CompactBinaryWriter::new();
        writer.write_bool(true);
        writer.write_int8(-1);
        writer.write_int16(2790);
        writer.write_int32(1);
        writer.write_uint64(300);
        writer.write_list_header(BondType::Int8, 3);
        assert_eq!(
            writer.into_bytes(),
            [0x01, 0xFF, 0xCC, 0x2B, 0x02, 0xAC, 0x02, 0x0E, 0x03]
        );
    }

    const STRUCT_WITH_UNKNOWN_FIELDS: [u8; 15] = [
        0x2E, 0x07, // field 1: int8 7 (unknown)
        0xC2, 0x0A, 0x01, // field 10: bool true (known)
        0xC9, 0x0F, 0x02, b'h', b'i', // field 15: string "hi" (unknown)
        0xE6, 0x2C, 0x01, 0x05, // field 300: uint64 5 (unknown)
        0x00, // stop
    ];

    fn decode(data: &[u8]) -> (bool, UnknownFields) {
        let mut reader = CompactBinaryReader::new(data);
        let mut flag = false;
        let mut unknown = UnknownFields::default();
        reader
            .read_fields(|reader, id, bond_type| match id {
                10 => {
                    expect_type(id, bond_type, BondType::Bool)?;
                    flag = reader.read_bool()?;
                    Ok(())
                }
                _ => unknown.capture(reader, id, bond_type),
            })
            .expect("fixture struct decodes");
        (flag, unknown)
    }

    fn encode(flag: bool, unknown: &UnknownFields) -> Vec<u8> {
        let mut writer = CompactBinaryWriter::new();
        writer.write_struct(unknown, |fields| {
            if flag {
                fields.bool(10, true);
            }
        });
        writer.into_bytes()
    }

    #[test]
    fn merges_unknown_fields_in_id_order() {
        let (flag, unknown) = decode(&STRUCT_WITH_UNKNOWN_FIELDS);
        assert!(flag);
        assert_eq!(encode(flag, &unknown), STRUCT_WITH_UNKNOWN_FIELDS);
    }

    #[test]
    fn keeps_unknown_fields_when_known_field_is_omitted() {
        let (_, unknown) = decode(&STRUCT_WITH_UNKNOWN_FIELDS);
        let mut expected = STRUCT_WITH_UNKNOWN_FIELDS.to_vec();
        expected.drain(2..5);
        assert_eq!(encode(false, &unknown), expected);
    }

    #[test]
    fn sorts_out_of_order_unknown_fields() {
        let data = [0xC9, 0x0F, 0x02, b'h', b'i', 0x2E, 0x07, 0x00];
        let (_, unknown) = decode(&data);
        assert_eq!(
            encode(false, &unknown),
            [0x2E, 0x07, 0xC9, 0x0F, 0x02, b'h', b'i', 0x00]
        );
    }
}
