use super::BondError;
use super::types::BondType;
use super::types::COMPACT_BINARY_MAGIC;
use super::types::COMPACT_BINARY_V1;
#[cfg(test)]
use super::value::BondStruct;
#[cfg(test)]
use super::value::BondValue;
use super::varint::decode_zigzag_i16;
use super::varint::decode_zigzag_i32;
#[cfg(test)]
use super::varint::decode_zigzag_i64;
use super::varint::read_varint;

/// Result of reading a field header: either a field with ID+type, or a struct
/// terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldHeader {
    Field { id: u16, bond_type: BondType },
    Stop,
    StopBase,
}

/// Deserializer for Bond `CompactBinary` v1 payloads.
pub(crate) struct CompactBinaryReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CompactBinaryReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_byte(&mut self) -> Result<u8, BondError> {
        let b = *self
            .data
            .get(self.pos)
            .ok_or(BondError::UnexpectedEof(self.pos))?;
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], BondError> {
        let slice = self
            .pos
            .checked_add(n)
            .and_then(|end| self.data.get(self.pos..end))
            .ok_or(BondError::UnexpectedEof(self.pos))?;
        self.pos += n;
        Ok(slice)
    }

    /// Reads exactly `n` bytes as a borrowed slice, advancing the cursor.
    pub(crate) fn read_bytes_slice(&mut self, n: usize) -> Result<&'a [u8], BondError> {
        self.read_bytes(n)
    }

    // -- Marshaled header --

    pub(crate) fn read_marshaled_header(&mut self) -> Result<(), BondError> {
        let magic = self.read_bytes(2)?;
        if magic != COMPACT_BINARY_MAGIC {
            return Err(BondError::InvalidHeader);
        }
        let version = self.read_bytes(2)?;
        if version != COMPACT_BINARY_V1 {
            return Err(BondError::InvalidHeader);
        }
        Ok(())
    }

    // -- Field headers --

    pub(crate) fn read_field_header(&mut self) -> Result<FieldHeader, BondError> {
        let raw = self.read_byte()?;

        let type_id = raw & 0x1F;
        let id_bits = raw & 0xE0; // upper 3 bits

        if type_id == 0 {
            return if id_bits == 0 {
                Ok(FieldHeader::Stop)
            } else {
                Err(BondError::InvalidTypeId(raw))
            };
        }

        if type_id == 1 && id_bits == 0 {
            return Ok(FieldHeader::StopBase);
        }

        let bond_type = BondType::try_from(type_id).map_err(BondError::InvalidTypeId)?;

        let id = match id_bits {
            0xE0 => {
                let lo = self.read_byte()?;
                let hi = self.read_byte()?;
                u16::from_le_bytes([lo, hi])
            }
            0xC0 => u16::from(self.read_byte()?),
            _ => u16::from(id_bits >> 5),
        };

        Ok(FieldHeader::Field { id, bond_type })
    }

    // -- Primitive readers --

    pub(crate) fn read_bool(&mut self) -> Result<bool, BondError> {
        Ok(self.read_byte()? != 0)
    }

    pub(crate) fn read_int8(&mut self) -> Result<i8, BondError> {
        Ok(self.read_byte()?.cast_signed())
    }

    pub(crate) fn read_int16(&mut self) -> Result<i16, BondError> {
        let (val, new_pos) = read_varint(self.data, self.pos)?;
        self.pos = new_pos;
        Ok(decode_zigzag_i16(u16::try_from(val)?))
    }

    pub(crate) fn read_uint32(&mut self) -> Result<u32, BondError> {
        let (val, new_pos) = read_varint(self.data, self.pos)?;
        self.pos = new_pos;
        Ok(u32::try_from(val)?)
    }

    pub(crate) fn read_int32(&mut self) -> Result<i32, BondError> {
        let (val, new_pos) = read_varint(self.data, self.pos)?;
        self.pos = new_pos;
        Ok(decode_zigzag_i32(u32::try_from(val)?))
    }

    pub(crate) fn read_uint64(&mut self) -> Result<u64, BondError> {
        let (val, new_pos) = read_varint(self.data, self.pos)?;
        self.pos = new_pos;
        Ok(val)
    }

    // -- Container headers --

    /// Reads a list or set header. Returns (`element_type`, count).
    pub(crate) fn read_container_header(&mut self) -> Result<(BondType, u32), BondError> {
        let raw = self.read_byte()?;
        let type_id = raw & 0x1F;
        let element_type = BondType::try_from(type_id).map_err(BondError::InvalidTypeId)?;
        let count = self.read_uint32()?;
        Ok((element_type, count))
    }

    /// Reads a map header. Returns (`key_type`, `value_type`, count).
    pub(crate) fn read_map_header(&mut self) -> Result<(BondType, BondType, u32), BondError> {
        let key_raw = self.read_byte()?;
        let key_type = BondType::try_from(key_raw & 0x1F).map_err(BondError::InvalidTypeId)?;
        let val_raw = self.read_byte()?;
        let val_type = BondType::try_from(val_raw & 0x1F).map_err(BondError::InvalidTypeId)?;
        let count = self.read_uint32()?;
        Ok((key_type, val_type, count))
    }

    // -- Skipping --

    /// Advances past a value of the given Bond type without allocating.
    pub(crate) fn skip_value(&mut self, bond_type: BondType) -> Result<(), BondError> {
        match bond_type {
            BondType::Bool | BondType::UInt8 | BondType::Int8 => {
                self.read_byte()?;
            }
            BondType::UInt16
            | BondType::UInt32
            | BondType::UInt64
            | BondType::Int16
            | BondType::Int32
            | BondType::Int64 => {
                let (_, new_pos) = read_varint(self.data, self.pos)?;
                self.pos = new_pos;
            }
            BondType::Float => {
                self.read_bytes(4)?;
            }
            BondType::Double => {
                self.read_bytes(8)?;
            }
            BondType::String => {
                let len = self.read_uint32()? as usize;
                self.read_bytes(len)?;
            }
            BondType::WString => {
                let len = self.read_uint32()? as usize;
                let byte_len = len.checked_mul(2).ok_or(BondError::VarintOverflow)?;
                self.read_bytes(byte_len)?;
            }
            BondType::Struct => {
                self.skip_struct()?;
            }
            BondType::List | BondType::Set => {
                let (element_type, count) = self.read_container_header()?;
                for _ in 0..count {
                    self.skip_value(element_type)?;
                }
            }
            BondType::Map => {
                let (key_type, value_type, count) = self.read_map_header()?;
                for _ in 0..count {
                    self.skip_value(key_type)?;
                    self.skip_value(value_type)?;
                }
            }
        }
        Ok(())
    }

    /// Advances past an entire struct (field headers + values) until `BT_STOP`,
    /// without allocating.
    pub(crate) fn skip_struct(&mut self) -> Result<(), BondError> {
        loop {
            match self.read_field_header()? {
                FieldHeader::Stop => return Ok(()),
                FieldHeader::StopBase => {}
                FieldHeader::Field { bond_type, .. } => {
                    self.skip_value(bond_type)?;
                }
            }
        }
    }
}

#[cfg(test)]
impl CompactBinaryReader<'_> {
    pub(super) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], BondError> {
        let pos = self.pos;
        self.read_bytes(N)?
            .first_chunk::<N>()
            .copied()
            .ok_or(BondError::UnexpectedEof(pos))
    }

    fn read_uint8(&mut self) -> Result<u8, BondError> {
        self.read_byte()
    }

    fn read_uint16(&mut self) -> Result<u16, BondError> {
        let (val, new_pos) = read_varint(self.data, self.pos)?;
        self.pos = new_pos;
        Ok(u16::try_from(val)?)
    }

    fn read_int64(&mut self) -> Result<i64, BondError> {
        let (val, new_pos) = read_varint(self.data, self.pos)?;
        self.pos = new_pos;
        Ok(decode_zigzag_i64(val))
    }

    fn read_float(&mut self) -> Result<f32, BondError> {
        Ok(f32::from_le_bytes(self.read_array()?))
    }

    fn read_double(&mut self) -> Result<f64, BondError> {
        Ok(f64::from_le_bytes(self.read_array()?))
    }

    fn read_string(&mut self) -> Result<String, BondError> {
        let len = self.read_uint32()? as usize;
        let bytes = self.read_bytes(len)?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }

    fn read_wstring(&mut self) -> Result<String, BondError> {
        let len = self.read_uint32()? as usize; // number of UTF-16 code units
        let byte_len = len.checked_mul(2).ok_or(BondError::VarintOverflow)?;
        let bytes = self.read_bytes(byte_len)?;
        let utf16: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        Ok(String::from_utf16(&utf16)?)
    }

    // -- High-level readers --

    /// Reads a single value of the given Bond type.
    fn read_value(&mut self, bond_type: BondType) -> Result<BondValue, BondError> {
        match bond_type {
            BondType::Bool => Ok(BondValue::Bool(self.read_bool()?)),
            BondType::UInt8 => Ok(BondValue::UInt8(self.read_uint8()?)),
            BondType::Int8 => Ok(BondValue::Int8(self.read_int8()?)),
            BondType::UInt16 => Ok(BondValue::UInt16(self.read_uint16()?)),
            BondType::Int16 => Ok(BondValue::Int16(self.read_int16()?)),
            BondType::UInt32 => Ok(BondValue::UInt32(self.read_uint32()?)),
            BondType::Int32 => Ok(BondValue::Int32(self.read_int32()?)),
            BondType::UInt64 => Ok(BondValue::UInt64(self.read_uint64()?)),
            BondType::Int64 => Ok(BondValue::Int64(self.read_int64()?)),
            BondType::Float => Ok(BondValue::Float(self.read_float()?)),
            BondType::Double => Ok(BondValue::Double(self.read_double()?)),
            BondType::String => Ok(BondValue::String(self.read_string()?)),
            BondType::WString => Ok(BondValue::WString(self.read_wstring()?)),
            BondType::Struct => Ok(BondValue::Struct(self.read_struct()?)),
            BondType::List => {
                let (element_type, count) = self.read_container_header()?;
                let mut elements = Vec::with_capacity((count as usize).min(self.remaining()));
                for _ in 0..count {
                    elements.push(self.read_value(element_type)?);
                }
                Ok(BondValue::List {
                    element_type,
                    elements,
                })
            }
            BondType::Set => {
                let (element_type, count) = self.read_container_header()?;
                let mut elements = Vec::with_capacity((count as usize).min(self.remaining()));
                for _ in 0..count {
                    elements.push(self.read_value(element_type)?);
                }
                Ok(BondValue::Set {
                    element_type,
                    elements,
                })
            }
            BondType::Map => {
                let (key_type, value_type, count) = self.read_map_header()?;
                let mut entries = Vec::with_capacity((count as usize).min(self.remaining()));
                for _ in 0..count {
                    let k = self.read_value(key_type)?;
                    let v = self.read_value(value_type)?;
                    entries.push((k, v));
                }
                Ok(BondValue::Map {
                    key_type,
                    value_type,
                    entries,
                })
            }
        }
    }

    /// Reads all fields of a struct until `BT_STOP`, returning a `BondStruct`.
    pub(super) fn read_struct(&mut self) -> Result<BondStruct, BondError> {
        let mut fields = Vec::new();
        loop {
            match self.read_field_header()? {
                FieldHeader::Stop => break,
                FieldHeader::StopBase => {}
                FieldHeader::Field { id, bond_type } => {
                    let value = self.read_value(bond_type)?;
                    fields.push((id, value));
                }
            }
        }
        Ok(BondStruct { fields })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_marshaled_header() {
        let data = [0x43, 0x42, 0x01, 0x00];
        let mut reader = CompactBinaryReader::new(&data);
        reader.read_marshaled_header().expect("header is valid");
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn read_marshaled_header_invalid_magic() {
        let data = [0x00, 0x00, 0x01, 0x00];
        let mut reader = CompactBinaryReader::new(&data);
        assert!(reader.read_marshaled_header().is_err());
    }

    #[test]
    fn read_field_header_small_ids() {
        let data = [0x02];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::Field {
                id: 0,
                bond_type: BondType::Bool
            }
        );

        let data = [0x2A];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::Field {
                id: 1,
                bond_type: BondType::Struct
            }
        );

        let data = [0xA6];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::Field {
                id: 5,
                bond_type: BondType::UInt64
            }
        );
    }

    #[test]
    fn read_field_header_extended_1byte() {
        let data = [0xC2, 0x0A];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::Field {
                id: 10,
                bond_type: BondType::Bool
            }
        );

        let data = [0xCF, 0x28];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::Field {
                id: 40,
                bond_type: BondType::Int16
            }
        );
    }

    #[test]
    fn read_field_header_extended_2byte() {
        let data = [0xE5, 0x2C, 0x01];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::Field {
                id: 300,
                bond_type: BondType::UInt32
            }
        );
    }

    #[test]
    fn read_stop() {
        let data = [0x00];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::Stop
        );
    }

    #[test]
    fn read_stop_base() {
        let data = [0x01];
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_field_header().expect("field header decodes"),
            FieldHeader::StopBase
        );
    }

    #[test]
    fn read_simple_struct() {
        let data = vec![
            0x02, // field 0, BT_BOOL
            0x01, // true
            0x26, // field 1, BT_UINT64
            0x2A, // varint(42)
            0x00, // BT_STOP
        ];

        let mut reader = CompactBinaryReader::new(&data);
        let s = reader.read_struct().expect("struct decodes");
        assert_eq!(s.fields.len(), 2);
        assert_eq!(s.fields[0], (0, BondValue::Bool(true)));
        assert_eq!(s.fields[1], (1, BondValue::UInt64(42)));
    }

    #[test]
    fn read_nested_struct() {
        let data = [
            0x0A, // field 0, BT_STRUCT
            0x0E, // field 0, BT_INT8
            0x07, // value = 7
            0x00, // BT_STOP (inner)
            0x00, // BT_STOP (outer)
        ];
        let mut reader = CompactBinaryReader::new(&data);
        let s = reader.read_struct().expect("struct decodes");
        assert_eq!(s.fields.len(), 1);
        if let BondValue::Struct(inner) = &s.fields[0].1 {
            assert_eq!(inner.fields.len(), 1);
            assert_eq!(inner.fields[0], (0, BondValue::Int8(7)));
        } else {
            panic!("expected struct");
        }
    }

    #[test]
    fn read_list() {
        let data = [
            0x0B, // field 0, BT_LIST
            0x0E, // element type = BT_INT8
            0x03, // count = 3
            0x01, 0x02, 0x03, // elements
            0x00, // BT_STOP
        ];
        let mut reader = CompactBinaryReader::new(&data);
        let s = reader.read_struct().expect("struct decodes");
        assert_eq!(s.fields.len(), 1);
        if let BondValue::List {
            element_type,
            elements,
        } = &s.fields[0].1
        {
            assert_eq!(*element_type, BondType::Int8);
            assert_eq!(elements.len(), 3);
            assert_eq!(elements[0], BondValue::Int8(1));
            assert_eq!(elements[1], BondValue::Int8(2));
            assert_eq!(elements[2], BondValue::Int8(3));
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn read_truncated_field_header() {
        let data = [];
        let mut reader = CompactBinaryReader::new(&data);
        assert!(reader.read_field_header().is_err());
    }

    #[test]
    fn read_truncated_extended_field_header() {
        let data = [0xC2];
        let mut reader = CompactBinaryReader::new(&data);
        assert!(reader.read_field_header().is_err());
    }

    #[test]
    fn read_truncated_varint() {
        let data = [0x80];
        let mut reader = CompactBinaryReader::new(&data);
        assert!(reader.read_uint64().is_err());
    }

    #[test]
    fn read_string_roundtrip() {
        use crate::bond::writer::CompactBinaryWriter;

        let mut w = CompactBinaryWriter::new();
        w.write_string("hello world")
            .expect("string length fits in u32");
        let bytes = w.into_bytes();

        let mut r = CompactBinaryReader::new(&bytes);
        assert_eq!(r.read_string().expect("string decodes"), "hello world");
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn read_float_double_roundtrip() {
        use crate::bond::writer::CompactBinaryWriter;

        let mut w = CompactBinaryWriter::new();
        w.write_float(std::f32::consts::PI);
        w.write_double(std::f64::consts::E);
        let bytes = w.into_bytes();

        let mut r = CompactBinaryReader::new(&bytes);
        assert!(
            (r.read_float().expect("float decodes") - std::f32::consts::PI).abs() < f32::EPSILON
        );
        assert!(
            (r.read_double().expect("double decodes") - std::f64::consts::E).abs() < f64::EPSILON
        );
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn read_container_count_exceeds_data() {
        let data = [
            0x0B, // field 0, BT_LIST
            0x0E, // element type = BT_INT8
            0xE8, 0x07, // count = 1000 (varint)
            0x01, 0x02, // only 2 elements
            0x00, // BT_STOP
        ];
        let mut reader = CompactBinaryReader::new(&data);
        assert!(reader.read_struct().is_err());
    }
}
