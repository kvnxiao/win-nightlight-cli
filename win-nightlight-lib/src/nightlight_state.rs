use chrono::Utc;

use crate::{
    consts::*,
    parser::{DeserializationError, parse_last_modified_timestamp_block, timestamp_to_bytes},
};

/// These constant bytes will exist if the nightlight state is enabled
const NIGHTLIGHT_STATE_ENABLED_BYTES: [u8; 2] = [0x10, 0x00];

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
/// * unknown bytes of size [REMAINING_DATA_BYTES_BODY_SIZE] with values that change over time
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
    remaining_data: Vec<u8>,
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

    /// Read the remaining data bytes and save it if we need to write it back
    fn parse_remaining_data_block(
        data: &[u8],
        pos: usize,
    ) -> Result<(Vec<u8>, usize), DeserializationError> {
        let remaining_data_bytes: &[u8] = &data[pos..data.len() - STRUCT_FOOTER_BYTES.len()];
        let remaining_data_vec = Vec::from(remaining_data_bytes);
        let len = remaining_data_vec.len();
        Ok((remaining_data_vec, pos + len))
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

    /// Serializes a [NightlightState] struct into a byte slice.
    /// See [NightlightState] for more information about the binary format.
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
        remaining_struct_bytes.extend_from_slice(&self.remaining_data);

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

#[cfg(test)]
mod tests {
    use super::*;

    const BYTES_DISABLED: [u8; 41] = [
        0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0x89, 0x95, 0xFC, 0xBE, 0x06,
        0x2A, 0x2B, 0x0E, 0x13, 0x43, 0x42, 0x01, 0x00, 0xD0, 0x0A, 0x02, 0xC6, 0x14, 0xA9, 0xF6,
        0xE2, 0xD3, 0xEF, 0xEA, 0xE6, 0xED, 0x01, 0x00, 0x00, 0x00, 0x00,
    ];
    const BYTES_ENABLED: [u8; 43] = [
        0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0x89, 0x95, 0xFC, 0xBE, 0x06,
        0x2A, 0x2B, 0x0E, 0x15, 0x43, 0x42, 0x01, 0x00, 0x10, 0x00, 0xD0, 0x0A, 0x02, 0xC6, 0x14,
        0xA9, 0xF6, 0xE2, 0xD3, 0xEF, 0xEA, 0xE6, 0xED, 0x01, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn test_serialize_to_bytes() {
        let state_disabled = NightlightState {
            timestamp: 1742670473,
            is_enabled: false,
            remaining_data: vec![
                0xD0, 0x0A, 0x02, 0xC6, 0x14, 0xA9, 0xF6, 0xE2, 0xD3, 0xEF, 0xEA, 0xE7, 0xED, 0x01,
            ],
        };
        let bytes_disabled = state_disabled.serialize_to_bytes();
        assert_eq!(bytes_disabled, BYTES_DISABLED);

        let state_enabled = NightlightState {
            timestamp: 1742670473,
            is_enabled: true,
            remaining_data: vec![
                0xD0, 0x0A, 0x02, 0xC6, 0x14, 0xA9, 0xF6, 0xE2, 0xD3, 0xEF, 0xEA, 0xE7, 0xED, 0x01,
            ],
        };
        let bytes_enabled = state_enabled.serialize_to_bytes();
        assert_eq!(bytes_enabled, BYTES_ENABLED);
    }

    #[test]
    fn test_deserialize_from_bytes() {
        let expected_state_disabled = NightlightState {
            timestamp: 1742670473,
            is_enabled: false,
            remaining_data: vec![
                0xD0, 0x0A, 0x02, 0xC6, 0x14, 0xA9, 0xF6, 0xE2, 0xD3, 0xEF, 0xEA, 0xE7, 0xED, 0x01,
            ],
        };
        let state_disabled = NightlightState::deserialize_from_bytes(&BYTES_DISABLED).unwrap();
        assert_eq!(state_disabled, expected_state_disabled);

        let expected_state_enabled = NightlightState {
            timestamp: 1742670473,
            is_enabled: true,
            remaining_data: vec![
                0xD0, 0x0A, 0x02, 0xC6, 0x14, 0xA9, 0xF6, 0xE2, 0xD3, 0xEF, 0xEA, 0xE7, 0xED, 0x01,
            ],
        };
        let state_enabled = NightlightState::deserialize_from_bytes(&BYTES_ENABLED).unwrap();
        assert_eq!(state_enabled, expected_state_enabled);
    }

    #[test]
    fn test_serde_roundtrip() {
        let state_disabled = NightlightState::deserialize_from_bytes(&BYTES_DISABLED).unwrap();
        let bytes = state_disabled.serialize_to_bytes();
        let state_deserialized = NightlightState::deserialize_from_bytes(&bytes).unwrap();
        assert_eq!(state_deserialized, state_disabled);

        let state_enabled = NightlightState::deserialize_from_bytes(&BYTES_ENABLED).unwrap();
        let bytes = state_enabled.serialize_to_bytes();
        let state_deserialized = NightlightState::deserialize_from_bytes(&bytes).unwrap();
        assert_eq!(state_deserialized, state_enabled);
    }
}
