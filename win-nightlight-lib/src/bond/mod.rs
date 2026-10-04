mod reader;
mod types;
mod unknown;
mod varint;
mod writer;

pub(crate) use reader::CompactBinaryReader;
pub(crate) use reader::expect_type;
use std::num::TryFromIntError;
use thiserror::Error;
pub(crate) use types::BondType;
pub(crate) use unknown::UnknownFields;
pub(crate) use writer::CompactBinaryWriter;
pub(crate) use writer::StructWriter;

#[derive(Error, Debug)]
pub(crate) enum BondError {
    #[error("unexpected end of data at byte {0}")]
    UnexpectedEof(usize),
    #[error("invalid compact binary v1 header")]
    InvalidHeader,
    #[error("invalid bond type id {0:#04x}")]
    InvalidTypeId(u8),
    #[error("varint does not fit in 64 bits")]
    VarintOverflow,
    #[error("integer out of range")]
    IntegerOutOfRange(#[from] TryFromIntError),
    #[error("structs nest deeper than {0} levels")]
    NestingTooDeep(usize),
    #[error("unsupported base struct terminator at byte {0}")]
    UnexpectedStopBase(usize),
    #[error("unexpected trailing data at byte {0}")]
    TrailingData(usize),
    #[error("missing required field {0}")]
    MissingField(u16),
    #[error("field {id} has type {found:?}, expected {expected:?}")]
    UnexpectedFieldType {
        id: u16,
        expected: BondType,
        found: BondType,
    },
    #[error("field {id} has invalid value {value}")]
    InvalidValue { id: u16, value: String },
}
