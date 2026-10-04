#[cfg(test)]
use super::BondError;
use super::types::BondType;
use super::types::COMPACT_BINARY_MAGIC;
use super::types::COMPACT_BINARY_V1;
#[cfg(test)]
use super::value::BondStruct;
#[cfg(test)]
use super::value::BondValue;
use super::varint::encode_zigzag_i16;
use super::varint::encode_zigzag_i32;
#[cfg(test)]
use super::varint::encode_zigzag_i64;
use super::varint::write_varint;

/// Serializer for Bond `CompactBinary` v1 payloads.
#[derive(Default)]
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

    // -- Marshaled header --

    pub(crate) fn write_marshaled_header(&mut self) {
        self.buf.extend_from_slice(&COMPACT_BINARY_MAGIC);
        self.buf.extend_from_slice(&COMPACT_BINARY_V1);
    }

    // -- Field headers --

    pub(crate) fn write_field_header(&mut self, id: u16, bond_type: BondType) {
        let type_byte = bond_type as u8;
        debug_assert_eq!(type_byte & 0x1F, type_byte);
        let [lo, hi] = id.to_le_bytes();

        if id <= 5 {
            self.buf.push(type_byte | (lo << 5));
        } else if id <= 0xFF {
            self.buf.push(type_byte | (0x06 << 5));
            self.buf.push(lo);
        } else {
            self.buf.push(type_byte | (0x07 << 5));
            self.buf.push(lo);
            self.buf.push(hi);
        }
    }

    pub(crate) fn write_stop(&mut self) {
        self.buf.push(0x00);
    }

    // -- Primitive writers --

    pub(crate) fn write_bool(&mut self, val: bool) {
        self.buf.push(u8::from(val));
    }

    pub(crate) fn write_int8(&mut self, val: i8) {
        self.buf.push(val.cast_unsigned());
    }

    /// Appends raw bytes directly to the output buffer.
    /// Useful for bulk-writing contiguous fixed-width elements (e.g.
    /// list<int8>).
    pub(crate) fn write_raw_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub(crate) fn write_int16(&mut self, val: i16) {
        write_varint(&mut self.buf, u64::from(encode_zigzag_i16(val)));
    }

    pub(crate) fn write_uint32(&mut self, val: u32) {
        write_varint(&mut self.buf, u64::from(val));
    }

    pub(crate) fn write_int32(&mut self, val: i32) {
        write_varint(&mut self.buf, u64::from(encode_zigzag_i32(val)));
    }

    pub(crate) fn write_uint64(&mut self, val: u64) {
        write_varint(&mut self.buf, val);
    }

    // -- Container headers --

    /// Writes a list or set header (v1 format: type byte + varint count).
    pub(crate) fn write_container_header(&mut self, element_type: BondType, count: u32) {
        self.buf.push(element_type as u8);
        self.write_uint32(count);
    }
}

#[cfg(test)]
impl CompactBinaryWriter {
    fn write_uint8(&mut self, val: u8) {
        self.buf.push(val);
    }

    fn write_uint16(&mut self, val: u16) {
        write_varint(&mut self.buf, u64::from(val));
    }

    fn write_int64(&mut self, val: i64) {
        write_varint(&mut self.buf, encode_zigzag_i64(val));
    }

    pub(super) fn write_float(&mut self, val: f32) {
        self.buf.extend_from_slice(&val.to_le_bytes());
    }

    pub(super) fn write_double(&mut self, val: f64) {
        self.buf.extend_from_slice(&val.to_le_bytes());
    }

    pub(super) fn write_string(&mut self, val: &str) -> Result<(), BondError> {
        self.write_uint32(u32::try_from(val.len())?);
        self.buf.extend_from_slice(val.as_bytes());
        Ok(())
    }

    fn write_wstring(&mut self, val: &str) -> Result<(), BondError> {
        let utf16: Vec<u16> = val.encode_utf16().collect();
        self.write_uint32(u32::try_from(utf16.len())?);
        for unit in &utf16 {
            self.buf.extend_from_slice(&unit.to_le_bytes());
        }
        Ok(())
    }

    /// Writes a map header (key type + value type + varint count).
    fn write_map_header(&mut self, key_type: BondType, value_type: BondType, count: u32) {
        self.buf.push(key_type as u8);
        self.buf.push(value_type as u8);
        self.write_uint32(count);
    }

    // -- High-level writers --

    /// Writes a single `BondValue`.
    fn write_value(&mut self, val: &BondValue) -> Result<(), BondError> {
        match val {
            BondValue::Bool(v) => self.write_bool(*v),
            BondValue::UInt8(v) => self.write_uint8(*v),
            BondValue::Int8(v) => self.write_int8(*v),
            BondValue::UInt16(v) => self.write_uint16(*v),
            BondValue::Int16(v) => self.write_int16(*v),
            BondValue::UInt32(v) => self.write_uint32(*v),
            BondValue::Int32(v) => self.write_int32(*v),
            BondValue::UInt64(v) => self.write_uint64(*v),
            BondValue::Int64(v) => self.write_int64(*v),
            BondValue::Float(v) => self.write_float(*v),
            BondValue::Double(v) => self.write_double(*v),
            BondValue::String(v) => self.write_string(v)?,
            BondValue::WString(v) => self.write_wstring(v)?,
            BondValue::Struct(s) => self.write_struct(s)?,
            BondValue::List {
                element_type,
                elements,
            }
            | BondValue::Set {
                element_type,
                elements,
            } => {
                self.write_container_header(*element_type, u32::try_from(elements.len())?);
                for elem in elements {
                    self.write_value(elem)?;
                }
            }
            BondValue::Map {
                key_type,
                value_type,
                entries,
            } => {
                self.write_map_header(*key_type, *value_type, u32::try_from(entries.len())?);
                for (k, v) in entries {
                    self.write_value(k)?;
                    self.write_value(v)?;
                }
            }
        }
        Ok(())
    }

    /// Writes a `BondStruct` (fields + `BT_STOP`).
    pub(super) fn write_struct(&mut self, s: &BondStruct) -> Result<(), BondError> {
        for (id, val) in &s.fields {
            self.write_field_header(*id, val.bond_type());
            self.write_value(val)?;
        }
        self.write_stop();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bond::reader::CompactBinaryReader;

    #[test]
    fn write_marshaled_header() {
        let mut w = CompactBinaryWriter::new();
        w.write_marshaled_header();
        assert_eq!(w.into_bytes(), [0x43, 0x42, 0x01, 0x00]);
    }

    #[test]
    fn field_header_small_ids() {
        let mut w = CompactBinaryWriter::new();
        w.write_field_header(0, BondType::Bool);
        w.write_field_header(1, BondType::Struct);
        w.write_field_header(5, BondType::UInt64);
        assert_eq!(w.into_bytes(), [0x02, 0x2A, 0xA6]);
    }

    #[test]
    fn field_header_extended_1byte() {
        let mut w = CompactBinaryWriter::new();
        w.write_field_header(10, BondType::Bool);
        w.write_field_header(40, BondType::Int16);
        assert_eq!(w.into_bytes(), [0xC2, 0x0A, 0xCF, 0x28]);
    }

    #[test]
    fn field_header_extended_2byte() {
        let mut w = CompactBinaryWriter::new();
        w.write_field_header(300, BondType::UInt32);
        assert_eq!(w.into_bytes(), [0xE5, 0x2C, 0x01]);
    }

    #[test]
    fn reader_writer_roundtrip_struct() {
        let original = BondStruct {
            fields: vec![
                (0, BondValue::Bool(true)),
                (1, BondValue::UInt64(1_742_540_908)),
                (10, BondValue::Int16(2790)),
                (
                    20,
                    BondValue::Struct(BondStruct {
                        fields: vec![(0, BondValue::Int8(19)), (1, BondValue::Int8(23))],
                    }),
                ),
            ],
        };

        let mut w = CompactBinaryWriter::new();
        w.write_struct(&original).expect("struct encodes");
        let bytes = w.into_bytes();

        let mut r = CompactBinaryReader::new(&bytes);
        let decoded = r.read_struct().expect("struct decodes");
        assert_eq!(r.remaining(), 0);
        assert_eq!(original, decoded);
    }

    #[test]
    fn reader_writer_roundtrip_list() {
        let original = BondStruct {
            fields: vec![(
                0,
                BondValue::List {
                    element_type: BondType::Int8,
                    elements: vec![BondValue::Int8(1), BondValue::Int8(2), BondValue::Int8(3)],
                },
            )],
        };

        let mut w = CompactBinaryWriter::new();
        w.write_struct(&original).expect("struct encodes");
        let bytes = w.into_bytes();

        let mut r = CompactBinaryReader::new(&bytes);
        let decoded = r.read_struct().expect("struct decodes");
        assert_eq!(original, decoded);
    }

    #[test]
    fn reader_writer_roundtrip_map() {
        let original = BondStruct {
            fields: vec![(
                0,
                BondValue::Map {
                    key_type: BondType::String,
                    value_type: BondType::Int32,
                    entries: vec![
                        (BondValue::String("hello".into()), BondValue::Int32(42)),
                        (BondValue::String("world".into()), BondValue::Int32(-1)),
                    ],
                },
            )],
        };

        let mut w = CompactBinaryWriter::new();
        w.write_struct(&original).expect("struct encodes");
        let bytes = w.into_bytes();

        let mut r = CompactBinaryReader::new(&bytes);
        let decoded = r.read_struct().expect("struct decodes");
        assert_eq!(original, decoded);
    }
}
