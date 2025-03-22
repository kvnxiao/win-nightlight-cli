use chrono::Utc;

use crate::{
    consts::*,
    parser::{DeserializationError, parse_last_modified_timestamp_block, timestamp_to_bytes},
};

/// These constant bytes will exist if the nightlight state is enabled
const NIGHTLIGHT_STATE_ENABLED_BYTES: [u8; 2] = [0x10, 0x00];
const REMAINING_DATA_BYTES_HEADER: [u8; 5] = [0xD0, 0x0A, 0x02, 0xC6, 0x14];
const REMAINING_DATA_BYTES_BODY_SIZE: usize = 6;
const REMAINING_DATA_BYTES_FOOTER: [u8; 3] = [0xE6, 0xED, 0x01];

/// The windows.data.bluelightreduction.bluelightreductionstate data structure has the following binary format:
///
/// * [STRUCT_HEADER_BYTES]
/// * [TIMESTAMP_HEADER_BYTES]
/// * [TIMESTAMP_PREFIX_BYTES]
/// * The last-modified Unix timestamp in seconds, variably-encoded into [TIMESTAMP_SIZE] bytes
///     - byte 0: bits 0-6 = timestamp's bits 0-6, but top bit 7 is always set
///     - byte 1: bits 0-6 = timestamp's bits 7-13, but top bit 7 is always set
///     - byte 2: bits 0-6 = timestamp's bits 14-20, but top bit 7 is always set
///     - byte 3: bits 0-6 = timestamp's bits 21-27, but top bit 7 is always set
///     - byte 4: bits 0-6 = timestamp's bits 28-31, but top bit 7 is NOT set
/// * [TIMESTAMP_SUFFIX_BYTES]
/// * single byte to identify the length of the remaining data
///     - the purpose of these remaining bytes is currently unknown, so the known values of this single byte are:
///         - 0x13: is_enabled = true
///         - 0x15: is_enabled = false
/// * [STRUCT_HEADER_BYTES] again
/// * if is_enabled = true, then include [NIGHTLIGHT_STATE_ENABLED_BYTES]
/// * [REMAINING_DATA_BYTES_HEADER]
/// * unknown bytes of size [REMAINING_DATA_BYTES_BODY_SIZE] with values that change over time
/// * [REMAINING_DATA_BYTES_FOOTER]
/// * [STRUCT_FOOTER_BYTES]
///
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NightlightState {
    /// The last-modified Unix timestamp in seconds
    pub timestamp: u64,
    /// Whether the nightlight is (force) enabled or not.
    /// If true, then the nightlight will be enabled regardless of the schedule settings.
    pub is_enabled: bool,
    /// The remaining data bytes read from the registry
    remaining_data: [u8; 6],
}

impl NightlightState {
    /// Parses the struct header block.
    fn parse_struct_header_block(data: &[u8], pos: usize) -> Result<usize, DeserializationError> {
        if data[pos..pos + STRUCT_HEADER_BYTES.len()] != STRUCT_HEADER_BYTES {
            return Err(DeserializationError::StructStart);
        }
        Ok(pos + STRUCT_HEADER_BYTES.len())
    }

    /// Parses the struct footer block.
    fn parse_struct_footer_block(data: &[u8], pos: usize) -> Result<usize, DeserializationError> {
        if data[pos..pos + STRUCT_FOOTER_BYTES.len()] != STRUCT_FOOTER_BYTES {
            return Err(DeserializationError::StructEnd);
        }
        Ok(pos + STRUCT_FOOTER_BYTES.len())
    }

    fn parse_is_enabled_block(data: &[u8], pos: usize) -> (bool, usize) {
        match data[pos..pos + NIGHTLIGHT_STATE_ENABLED_BYTES.len()]
            == NIGHTLIGHT_STATE_ENABLED_BYTES
        {
            true => (true, pos + NIGHTLIGHT_STATE_ENABLED_BYTES.len()),
            false => (false, pos),
        }
    }

    fn parse_remaining_data_block(
        data: &[u8],
        pos: usize,
    ) -> Result<([u8; 6], usize), DeserializationError> {
        let mut pos = pos;
        if data[pos..pos + REMAINING_DATA_BYTES_HEADER.len()] != REMAINING_DATA_BYTES_HEADER {
            return Err(DeserializationError::InvalidBlock(
                "RemainingDataHeader".into(),
            ));
        }
        pos += REMAINING_DATA_BYTES_HEADER.len();

        // Read the remaining data bytes and save it if we need to write it back
        let remaining_data_bytes: [u8; 6] = data[pos..pos + REMAINING_DATA_BYTES_BODY_SIZE]
            .try_into()
            .map_err(|_| DeserializationError::InvalidBlock("RemainingDataBody".into()))?;
        pos += REMAINING_DATA_BYTES_BODY_SIZE;

        if data[pos..pos + REMAINING_DATA_BYTES_FOOTER.len()] != REMAINING_DATA_BYTES_FOOTER {
            return Err(DeserializationError::InvalidBlock(
                "RemainingDataFooter".into(),
            ));
        }
        pos += REMAINING_DATA_BYTES_FOOTER.len();
        Ok((remaining_data_bytes, pos))
    }

    /// Deserializes a [NightlightState] struct from a byte slice.
    /// See [NightlightState] for more information about the binary format.
    pub fn deserialize_from_bytes(data: &[u8]) -> Result<NightlightState, DeserializationError> {
        let pos = Self::parse_struct_header_block(data, 0)?;
        let (timestamp, pos) = parse_last_modified_timestamp_block(data, pos)?;

        // Check if the remaining struct size is valid
        let remaining_struct_size: u8 = data[pos];
        if data.len() != remaining_struct_size as usize + pos + STRUCT_FOOTER_BYTES.len() {
            return Err(DeserializationError::StructEnd);
        }

        let pos = Self::parse_struct_header_block(data, pos + 1)?; // skip 1 byte since we read remaining_struct_size
        let (is_enabled, pos) = Self::parse_is_enabled_block(data, pos);
        let (remaining_data, pos) = Self::parse_remaining_data_block(data, pos)?;
        let pos = Self::parse_struct_footer_block(data, pos)?;

        if pos != data.len() {
            return Err(DeserializationError::StructEnd);
        }

        Ok(NightlightState {
            timestamp,
            is_enabled,
            remaining_data,
        })
    }

    pub fn serialize_to_bytes(&self) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(&STRUCT_HEADER_BYTES);
        bytes.extend_from_slice(&TIMESTAMP_HEADER_BYTES);
        bytes.extend_from_slice(&TIMESTAMP_PREFIX_BYTES);
        let timestamp_bytes = timestamp_to_bytes(self.timestamp);
        bytes.extend_from_slice(&timestamp_bytes);
        bytes.extend_from_slice(&TIMESTAMP_SUFFIX_BYTES);

        // Figure out the size of the remaining bytes after computing the back bytes
        let mut remaining_struct_bytes: Vec<u8> = Vec::new();
        remaining_struct_bytes.extend_from_slice(&STRUCT_HEADER_BYTES);
        if self.is_enabled {
            remaining_struct_bytes.extend_from_slice(&NIGHTLIGHT_STATE_ENABLED_BYTES);
        }
        remaining_struct_bytes.extend_from_slice(&REMAINING_DATA_BYTES_HEADER);
        remaining_struct_bytes.extend_from_slice(&self.remaining_data);
        remaining_struct_bytes.extend_from_slice(&REMAINING_DATA_BYTES_FOOTER);

        let remaining_struct_size = remaining_struct_bytes.len() as u8 + 1;
        bytes.push(remaining_struct_size);
        bytes.extend(remaining_struct_bytes);
        bytes.extend_from_slice(&STRUCT_FOOTER_BYTES);
        bytes
    }

    fn update_timestamp(&mut self) {
        self.timestamp = Utc::now().timestamp() as u64;
    }

    /// Enables the nightlight and updates the timestamp.
    /// Returns true if a change was made (i.e. the nightlight was previously disabled).
    pub fn enable(&mut self) -> bool {
        match !self.is_enabled {
            true => {
                self.is_enabled = true;
                self.update_timestamp();
                true
            }
            false => false,
        }
    }

    /// Disables the nightlight and updates the timestamp.
    /// Returns true if a change was made (i.e. the nightlight was previously enabled).
    pub fn disable(&mut self) -> bool {
        match self.is_enabled {
            true => {
                self.is_enabled = false;
                self.update_timestamp();
                true
            }
            false => false,
        }
    }
}
