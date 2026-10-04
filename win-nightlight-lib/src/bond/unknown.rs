use super::BondError;
use super::BondType;
use super::CompactBinaryReader;

/// Fields a decoder does not model, kept as encoded bytes in ascending ID
/// order so an encoder can write them back unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UnknownFields(Vec<UnknownField>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UnknownField {
    pub(super) id: u16,
    pub(super) bond_type: BondType,
    pub(super) value: Vec<u8>,
}

impl UnknownFields {
    pub(crate) fn capture(
        &mut self,
        reader: &mut CompactBinaryReader<'_>,
        id: u16,
        bond_type: BondType,
    ) -> Result<(), BondError> {
        let value = reader.read_raw_value(bond_type)?.to_vec();
        let at = self.0.partition_point(|field| field.id <= id);
        self.0.insert(
            at,
            UnknownField {
                id,
                bond_type,
                value,
            },
        );
        Ok(())
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, UnknownField> {
        self.0.iter()
    }
}
