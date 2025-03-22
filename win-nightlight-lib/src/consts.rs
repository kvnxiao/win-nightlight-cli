/// Identifies the beginning of the struct
pub const STRUCT_HEADER_BYTES: [u8; 4] = [0x43, 0x42, 0x01, 0x00];
/// Identifies the end of the struct
pub const STRUCT_FOOTER_BYTES: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// Identifies the beginning of the timestamp struct
pub const TIMESTAMP_HEADER_BYTES: [u8; 4] = [0x0A, 0x02, 0x01, 0x00];
/// Identifies the start of the timestamp definition, and is always followed by the actual timestamp value
pub const TIMESTAMP_PREFIX_BYTES: [u8; 2] = [0x2A, 0x06];
/// The size of the timestamp struct in bytes
pub const TIMESTAMP_SIZE: usize = 5;
/// Identifies the end of the timestamp definition, and will always be preceded by the timestamp value
pub const TIMESTAMP_SUFFIX_BYTES: [u8; 3] = [0x2A, 0x2B, 0x0E];
