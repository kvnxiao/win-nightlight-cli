mod types;
mod varint;

pub(crate) mod reader;
#[cfg(test)]
pub(crate) mod value;
pub(crate) mod writer;

pub(crate) use reader::CompactBinaryReader;
pub(crate) use reader::FieldHeader;
use std::num::TryFromIntError;
use std::string::FromUtf8Error;
use std::string::FromUtf16Error;
use thiserror::Error;
pub(crate) use types::BondType;
pub(crate) use writer::CompactBinaryWriter;

/// Failure to decode or encode a Bond `CompactBinary` v1 payload.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum BondError {
    /// The payload ended before the value at this byte offset was complete.
    #[error("Unexpected end of data at position {0}")]
    UnexpectedEof(usize),
    /// The payload does not start with the `CompactBinary` v1 magic and
    /// version.
    #[error("Invalid marshaled header")]
    InvalidHeader,
    /// A field or container header names an unknown Bond type ID.
    #[error("Invalid type ID: {0}")]
    InvalidTypeId(u8),
    /// A varint does not fit in 64 bits.
    #[error("Varint overflow")]
    VarintOverflow,
    /// An integer does not fit the range of its Bond or Rust target type.
    #[error("Integer out of range")]
    IntegerOutOfRange(#[from] TryFromIntError),
    /// A `string` value is not valid UTF-8.
    #[error("Invalid UTF-8 string")]
    InvalidUtf8(#[from] FromUtf8Error),
    /// A `wstring` value is not valid UTF-16.
    #[error("Invalid UTF-16 string")]
    InvalidUtf16(#[from] FromUtf16Error),
    /// A required field with this ID is absent.
    #[error("Missing required field {0}")]
    MissingField(u16),
    /// The field with this ID has an unexpected type or value.
    #[error("Unexpected field type for field {0}")]
    UnexpectedFieldType(u16),
}
