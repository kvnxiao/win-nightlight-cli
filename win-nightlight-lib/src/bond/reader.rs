use super::BondError;
use super::types::BondType;
use super::types::COMPACT_BINARY_MAGIC;
use super::types::COMPACT_BINARY_V1;
use super::varint::decode_zigzag_i16;
use super::varint::decode_zigzag_i32;
use super::varint::read_varint;

const MAX_NESTING_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldHeader {
    Field { id: u16, bond_type: BondType },
    Stop,
    StopBase,
}

pub(crate) fn expect_type(id: u16, found: BondType, expected: BondType) -> Result<(), BondError> {
    if found == expected {
        Ok(())
    } else {
        Err(BondError::UnexpectedFieldType {
            id,
            expected,
            found,
        })
    }
}

pub(crate) struct CompactBinaryReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CompactBinaryReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn read_marshaled_header(&mut self) -> Result<(), BondError> {
        if self.read_bytes(2)? != COMPACT_BINARY_MAGIC || self.read_bytes(2)? != COMPACT_BINARY_V1 {
            return Err(BondError::InvalidHeader);
        }
        Ok(())
    }

    pub(crate) fn expect_end(&self) -> Result<(), BondError> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(BondError::TrailingData(self.pos))
        }
    }

    /// `on_field` receives each field ID and type and must consume exactly
    /// that field's value.
    pub(crate) fn read_fields(
        &mut self,
        mut on_field: impl FnMut(&mut Self, u16, BondType) -> Result<(), BondError>,
    ) -> Result<(), BondError> {
        loop {
            let start = self.pos;
            match self.read_field_header()? {
                FieldHeader::Stop => return Ok(()),
                // Re-encoding cannot reproduce an inheritance boundary, so
                // accepting one would silently flatten the struct on write.
                FieldHeader::StopBase => return Err(BondError::UnexpectedStopBase(start)),
                FieldHeader::Field { id, bond_type } => on_field(self, id, bond_type)?,
            }
        }
    }

    pub(crate) fn read_raw_value(&mut self, bond_type: BondType) -> Result<&'a [u8], BondError> {
        let start = self.pos;
        self.skip_value(bond_type, 0)?;
        self.data
            .get(start..self.pos)
            .ok_or(BondError::UnexpectedEof(start))
    }

    pub(crate) fn read_bool(&mut self) -> Result<bool, BondError> {
        Ok(self.read_byte()? != 0)
    }

    pub(crate) fn read_int8(&mut self) -> Result<i8, BondError> {
        Ok(self.read_byte()?.cast_signed())
    }

    pub(crate) fn read_int16(&mut self) -> Result<i16, BondError> {
        Ok(decode_zigzag_i16(u16::try_from(self.read_varint()?)?))
    }

    pub(crate) fn read_int32(&mut self) -> Result<i32, BondError> {
        Ok(decode_zigzag_i32(u32::try_from(self.read_varint()?)?))
    }

    pub(crate) fn read_uint64(&mut self) -> Result<u64, BondError> {
        self.read_varint()
    }

    pub(crate) fn read_list_header(&mut self) -> Result<(BondType, u32), BondError> {
        let element_type = self.read_type_byte()?;
        let count = u32::try_from(self.read_varint()?)?;
        Ok((element_type, count))
    }

    pub(crate) fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], BondError> {
        let slice = self
            .pos
            .checked_add(len)
            .and_then(|end| self.data.get(self.pos..end))
            .ok_or(BondError::UnexpectedEof(self.data.len()))?;
        self.pos += len;
        Ok(slice)
    }

    fn read_byte(&mut self) -> Result<u8, BondError> {
        let byte = *self
            .data
            .get(self.pos)
            .ok_or(BondError::UnexpectedEof(self.pos))?;
        self.pos += 1;
        Ok(byte)
    }

    fn read_varint(&mut self) -> Result<u64, BondError> {
        let (value, pos) = read_varint(self.data, self.pos)?;
        self.pos = pos;
        Ok(value)
    }

    fn read_type_byte(&mut self) -> Result<BondType, BondError> {
        let raw = self.read_byte()?;
        BondType::try_from(raw & 0x1F).map_err(BondError::InvalidTypeId)
    }

    fn read_field_header(&mut self) -> Result<FieldHeader, BondError> {
        let raw = self.read_byte()?;
        let type_id = raw & 0x1F;
        let id_bits = raw >> 5;

        match (type_id, id_bits) {
            (0, 0) => return Ok(FieldHeader::Stop),
            (1, 0) => return Ok(FieldHeader::StopBase),
            _ => {}
        }

        let bond_type = BondType::try_from(type_id).map_err(BondError::InvalidTypeId)?;
        let id = match id_bits {
            7 => u16::from_le_bytes([self.read_byte()?, self.read_byte()?]),
            6 => u16::from(self.read_byte()?),
            small => u16::from(small),
        };
        Ok(FieldHeader::Field { id, bond_type })
    }

    fn skip_value(&mut self, bond_type: BondType, depth: usize) -> Result<(), BondError> {
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
                self.read_varint()?;
            }
            BondType::Float => {
                self.read_bytes(4)?;
            }
            BondType::Double => {
                self.read_bytes(8)?;
            }
            BondType::String => {
                let len = usize::try_from(self.read_varint()?)?;
                self.read_bytes(len)?;
            }
            BondType::WString => {
                let units = usize::try_from(self.read_varint()?)?;
                let len = units
                    .checked_mul(2)
                    .ok_or(BondError::UnexpectedEof(self.pos))?;
                self.read_bytes(len)?;
            }
            BondType::Struct => self.skip_struct(Self::nest(depth)?)?,
            BondType::List | BondType::Set => {
                let depth = Self::nest(depth)?;
                let (element_type, count) = self.read_list_header()?;
                for _ in 0..count {
                    self.skip_value(element_type, depth)?;
                }
            }
            BondType::Map => {
                let depth = Self::nest(depth)?;
                let key_type = self.read_type_byte()?;
                let value_type = self.read_type_byte()?;
                let count = u32::try_from(self.read_varint()?)?;
                for _ in 0..count {
                    self.skip_value(key_type, depth)?;
                    self.skip_value(value_type, depth)?;
                }
            }
        }
        Ok(())
    }

    fn skip_struct(&mut self, depth: usize) -> Result<(), BondError> {
        loop {
            match self.read_field_header()? {
                FieldHeader::Stop => return Ok(()),
                FieldHeader::StopBase => {}
                FieldHeader::Field { bond_type, .. } => self.skip_value(bond_type, depth)?,
            }
        }
    }

    fn nest(depth: usize) -> Result<usize, BondError> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(BondError::NestingTooDeep(MAX_NESTING_DEPTH));
        }
        Ok(depth + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    fn field_headers(data: &[u8]) -> Vec<(u16, BondType)> {
        let mut reader = CompactBinaryReader::new(data);
        let mut fields = Vec::new();
        reader
            .read_fields(|reader, id, bond_type| {
                fields.push((id, bond_type));
                reader.read_raw_value(bond_type).map(drop)
            })
            .expect("fixture struct decodes");
        reader.expect_end().expect("fixture has no trailing data");
        fields
    }

    #[test]
    fn reads_marshaled_header() {
        let mut reader = CompactBinaryReader::new(&[0x43, 0x42, 0x01, 0x00]);
        reader.read_marshaled_header().expect("header is valid");
        reader.expect_end().expect("header consumes all bytes");
    }

    #[test_case(&[0x00, 0x00, 0x01, 0x00]; "wrong magic")]
    #[test_case(&[0x43, 0x42, 0x02, 0x00]; "wrong version")]
    #[test_case(&[0x43, 0x42]; "truncated")]
    fn rejects_marshaled_header(data: &[u8]) {
        let result = CompactBinaryReader::new(data).read_marshaled_header();
        assert!(result.is_err(), "{result:?}");
    }

    #[test_case(&[0x02, 0x01, 0x00], &[(0, BondType::Bool)]; "small id")]
    #[test_case(&[0x2A, 0x00, 0x00], &[(1, BondType::Struct)]; "id one struct")]
    #[test_case(&[0xA6, 0x07, 0x00], &[(5, BondType::UInt64)]; "largest small id")]
    #[test_case(&[0xC2, 0x0A, 0x00, 0x00], &[(10, BondType::Bool)]; "one byte id")]
    #[test_case(&[0xCF, 0x28, 0xCC, 0x2B, 0x00], &[(40, BondType::Int16)]; "one byte id int16")]
    #[test_case(&[0xE5, 0x2C, 0x01, 0x05, 0x00], &[(300, BondType::UInt32)]; "two byte id")]
    fn reads_field_headers(data: &[u8], expected: &[(u16, BondType)]) {
        assert_eq!(field_headers(data), expected);
    }

    #[test_case(BondType::Bool, &[0x01])]
    #[test_case(BondType::Int8, &[0xFF])]
    #[test_case(BondType::UInt64, &[0xEC, 0xA0, 0xF4, 0xBE, 0x06])]
    #[test_case(BondType::Int16, &[0xCC, 0x2B])]
    #[test_case(BondType::Float, &[0x00, 0x00, 0x80, 0x3F])]
    #[test_case(BondType::Double, &[0, 0, 0, 0, 0, 0, 0xF0, 0x3F])]
    #[test_case(BondType::String, &[0x02, b'h', b'i'])]
    #[test_case(BondType::WString, &[0x02, b'h', 0x00, b'i', 0x00])]
    #[test_case(BondType::Struct, &[0x0A, 0x0E, 0x07, 0x00, 0x00])]
    #[test_case(BondType::List, &[0x0E, 0x03, 0x01, 0x02, 0x03])]
    #[test_case(BondType::Set, &[0x10, 0x02, 0x02, 0x04])]
    #[test_case(BondType::Map, &[0x09, 0x10, 0x01, 0x01, b'k', 0x54])]
    fn raw_value_spans_exactly_one_value(bond_type: BondType, encoded: &[u8]) {
        let mut data = encoded.to_vec();
        data.push(0xAA);
        let mut reader = CompactBinaryReader::new(&data);
        assert_eq!(
            reader.read_raw_value(bond_type).expect("value decodes"),
            encoded
        );
        assert_eq!(reader.read_bytes(1).expect("sentinel remains"), [0xAA]);
    }

    #[test]
    fn skips_base_struct_terminator_inside_raw_values() {
        let data = [0x0A, 0x02, 0x01, 0x01, 0x22, 0x00, 0x00, 0x00];
        assert_eq!(field_headers(&data), [(0, BondType::Struct)]);
    }

    #[test]
    fn rejects_base_struct_terminator_in_read_fields() {
        let data = [0x02, 0x01, 0x01, 0x00];
        let mut reader = CompactBinaryReader::new(&data);
        let result =
            reader.read_fields(|reader, _, bond_type| reader.read_raw_value(bond_type).map(drop));
        assert!(
            matches!(result, Err(BondError::UnexpectedStopBase(2))),
            "{result:?}"
        );
    }

    #[test_case(&[]; "empty")]
    #[test_case(&[0xC2]; "truncated one byte id")]
    #[test_case(&[0xE2, 0x01]; "truncated two byte id")]
    #[test_case(&[0x06, 0x80]; "truncated varint")]
    #[test_case(&[0x0B, 0x0E, 0xE8, 0x07, 0x01, 0x02, 0x00]; "list count exceeds data")]
    #[test_case(&[0x13, 0x00]; "unknown type id")]
    #[test_case(&[0x20, 0x00]; "stop with id bits")]
    fn rejects_malformed_struct(data: &[u8]) {
        let mut reader = CompactBinaryReader::new(data);
        let result =
            reader.read_fields(|reader, _, bond_type| reader.read_raw_value(bond_type).map(drop));
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn rejects_excessive_nesting() {
        let mut data = vec![0x0A; MAX_NESTING_DEPTH + 1];
        data.extend(std::iter::repeat_n(0x00, MAX_NESTING_DEPTH + 2));
        let mut reader = CompactBinaryReader::new(&data);
        let result =
            reader.read_fields(|reader, _, bond_type| reader.read_raw_value(bond_type).map(drop));
        assert!(
            matches!(result, Err(BondError::NestingTooDeep(MAX_NESTING_DEPTH))),
            "{result:?}"
        );
    }

    #[test]
    fn accepts_maximum_nesting() {
        let mut data = vec![0x0A; MAX_NESTING_DEPTH];
        data.extend(std::iter::repeat_n(0x00, MAX_NESTING_DEPTH + 1));
        assert_eq!(field_headers(&data), [(0, BondType::Struct)]);
    }

    #[test]
    fn rejects_trailing_data() {
        let mut reader = CompactBinaryReader::new(&[0x00, 0x00]);
        reader
            .read_fields(|_, _, _| Ok(()))
            .expect("empty struct decodes");
        assert!(matches!(
            reader.expect_end(),
            Err(BondError::TrailingData(1))
        ));
    }

    #[test]
    fn expect_type_reports_field_and_types() {
        let error = expect_type(40, BondType::Int32, BondType::Int16)
            .expect_err("mismatched type is rejected");
        assert_eq!(error.to_string(), "field 40 has type Int32, expected Int16");
    }
}
