use chrono::Utc;

use crate::bond::*;
use crate::cloudstore;

/// Night Light state stored in the registry as a Bond CompactBinary v1 payload.
///
/// The binary format is a CloudStore wrapper containing an inner Bond struct with fields:
/// - Field 0:  int32  — enabled flag (presence = force-enabled)
/// - Field 10: int32  — initialized marker (always 1)
/// - Field 20: uint64 — last transition FILETIME
///
/// See `docs/nightlight-registry-format.md` for full details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NightlightState {
    /// The last-modified Unix timestamp in seconds
    pub timestamp: u64,
    /// Whether the nightlight is (force) enabled or not.
    /// If true, then the nightlight will be enabled regardless of the schedule settings.
    pub is_enabled: bool,
    /// Always 1; likely a "data valid" or schema version marker.
    pub initialized: i32,
    /// Windows FILETIME of the last state transition (toggle or scheduled change).
    /// 100-nanosecond intervals since 1601-01-01 UTC.
    pub last_transition_filetime: u64,
}

impl NightlightState {
    /// Deserializes a [NightlightState] struct from a byte slice.
    pub fn deserialize_from_bytes(data: &[u8]) -> Result<NightlightState, BondError> {
        let (timestamp, inner_payload) = cloudstore::cloudstore_unwrap(data)?;

        let mut reader = CompactBinaryReader::new(&inner_payload);
        reader.read_marshaled_header()?;

        let mut is_enabled = false;
        let mut initialized: i32 = 0;
        let mut last_transition_filetime: u64 = 0;

        loop {
            match reader.read_field_header()? {
                FieldHeader::Stop => break,
                FieldHeader::StopBase => continue,
                FieldHeader::Field {
                    id: 0,
                    bond_type: BondType::Int32,
                } => {
                    let _ = reader.read_int32()?;
                    is_enabled = true; // presence is the signal
                }
                FieldHeader::Field {
                    id: 10,
                    bond_type: BondType::Int32,
                } => {
                    initialized = reader.read_int32()?;
                }
                FieldHeader::Field {
                    id: 20,
                    bond_type: BondType::UInt64,
                } => {
                    last_transition_filetime = reader.read_uint64()?;
                }
                FieldHeader::Field { bond_type, .. } => {
                    reader.skip_value(bond_type)?;
                }
            }
        }

        Ok(NightlightState {
            timestamp,
            is_enabled,
            initialized,
            last_transition_filetime,
        })
    }

    /// Serializes a [NightlightState] struct into a byte slice.
    pub fn serialize_to_bytes(&self) -> Vec<u8> {
        let mut inner = CompactBinaryWriter::new();
        inner.write_marshaled_header();

        // Field 0: enabled flag (only present when enabled)
        if self.is_enabled {
            inner.write_field_header(0, BondType::Int32);
            inner.write_int32(0);
        }

        // Field 10: initialized marker
        inner.write_field_header(10, BondType::Int32);
        inner.write_int32(self.initialized);

        // Field 20: last transition FILETIME
        inner.write_field_header(20, BondType::UInt64);
        inner.write_uint64(self.last_transition_filetime);

        inner.write_stop();

        cloudstore::cloudstore_wrap(self.timestamp, &inner.into_bytes())
    }

    fn update_timestamp(&mut self) {
        self.timestamp = Utc::now().timestamp() as u64;
    }

    /// Enables the nightlight and updates the timestamp.
    /// Returns true if a change was made (i.e. the nightlight was previously disabled).
    pub fn enable(&mut self) -> bool {
        if self.is_enabled {
            return false;
        }
        self.is_enabled = true;
        self.update_timestamp();
        true
    }

    /// Disables the nightlight and updates the timestamp.
    /// Returns true if a change was made (i.e. the nightlight was previously enabled).
    pub fn disable(&mut self) -> bool {
        if !self.is_enabled {
            return false;
        }
        self.is_enabled = false;
        self.update_timestamp();
        true
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

    // Decode the FILETIME from the test data varint bytes [0xA9, 0xF6, 0xE2, 0xD3, 0xEF, 0xEA, 0xE6, 0xED, 0x01]
    const EXPECTED_FILETIME: u64 = 133_871_411_809_270_569;

    fn expected_disabled() -> NightlightState {
        NightlightState {
            timestamp: 1742670473,
            is_enabled: false,
            initialized: 1,
            last_transition_filetime: EXPECTED_FILETIME,
        }
    }

    fn expected_enabled() -> NightlightState {
        NightlightState {
            timestamp: 1742670473,
            is_enabled: true,
            initialized: 1,
            last_transition_filetime: EXPECTED_FILETIME,
        }
    }

    #[test]
    fn test_serialize_to_bytes() {
        let bytes_disabled = expected_disabled().serialize_to_bytes();
        assert_eq!(bytes_disabled, BYTES_DISABLED);

        let bytes_enabled = expected_enabled().serialize_to_bytes();
        assert_eq!(bytes_enabled, BYTES_ENABLED);
    }

    #[test]
    fn test_deserialize_from_bytes() {
        let state_disabled = NightlightState::deserialize_from_bytes(&BYTES_DISABLED).unwrap();
        assert_eq!(state_disabled, expected_disabled());

        let state_enabled = NightlightState::deserialize_from_bytes(&BYTES_ENABLED).unwrap();
        assert_eq!(state_enabled, expected_enabled());
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
