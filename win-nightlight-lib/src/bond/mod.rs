#[allow(unused)]
mod types;
#[allow(unused)]
mod varint;

#[allow(unused)]
pub mod reader;
#[allow(unused)]
pub mod value;
#[allow(unused)]
pub mod writer;

pub use reader::{CompactBinaryReader, FieldHeader};
pub use types::BondType;
#[allow(unused_imports)]
pub use value::{BondStruct, BondValue};
pub use writer::CompactBinaryWriter;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum BondError {
    #[error("Unexpected end of data at position {0}")]
    UnexpectedEof(usize),
    #[error("Invalid marshaled header")]
    InvalidHeader,
    #[error("Invalid type ID: {0}")]
    InvalidTypeId(u8),
    #[error("Varint overflow")]
    VarintOverflow,
    #[error("Invalid UTF-8 string")]
    InvalidUtf8,
    #[error("Invalid UTF-16 string")]
    InvalidUtf16,
    #[error("Missing required field {0}")]
    MissingField(u16),
    #[error("Unexpected field type for field {0}")]
    UnexpectedFieldType(u16),
}
