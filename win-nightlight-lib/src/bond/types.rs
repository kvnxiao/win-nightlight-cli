pub(super) const COMPACT_BINARY_MAGIC: [u8; 2] = [0x43, 0x42];
pub(super) const COMPACT_BINARY_V1: [u8; 2] = [0x01, 0x00];

/// Bond data type identifier used in field and container headers.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BondType {
    Bool = 2,
    UInt8 = 3,
    UInt16 = 4,
    UInt32 = 5,
    UInt64 = 6,
    Float = 7,
    Double = 8,
    String = 9,
    Struct = 10,
    List = 11,
    Set = 12,
    Map = 13,
    Int8 = 14,
    Int16 = 15,
    Int32 = 16,
    Int64 = 17,
    WString = 18,
}

impl BondType {
    pub(super) fn id(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for BondType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            2 => Ok(Self::Bool),
            3 => Ok(Self::UInt8),
            4 => Ok(Self::UInt16),
            5 => Ok(Self::UInt32),
            6 => Ok(Self::UInt64),
            7 => Ok(Self::Float),
            8 => Ok(Self::Double),
            9 => Ok(Self::String),
            10 => Ok(Self::Struct),
            11 => Ok(Self::List),
            12 => Ok(Self::Set),
            13 => Ok(Self::Map),
            14 => Ok(Self::Int8),
            15 => Ok(Self::Int16),
            16 => Ok(Self::Int32),
            17 => Ok(Self::Int64),
            18 => Ok(Self::WString),
            _ => Err(value),
        }
    }
}
