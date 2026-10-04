use crate::bond::BondError;
use crate::bond::BondType;
use crate::bond::CompactBinaryReader;
use crate::bond::CompactBinaryWriter;
use crate::bond::UnknownFields;
use crate::bond::expect_type;
use crate::time::unix_secs_from_system_time;
use std::time::SystemTime;

const OUTER_CONTAINER: u16 = 1;
const CONTAINER_MODIFIED: u16 = 0;
const CONTAINER_WRAPPER: u16 = 1;
const WRAPPER_PAYLOAD: u16 = 1;
const MIN_MODIFIED_STEP_SECS: u64 = 2;

/// `CloudStore` wrapper around one Night Light payload.
///
/// Every level keeps the fields it does not model, including the outer
/// metadata struct, so a decoded envelope encodes back to identical bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Envelope {
    modified: u64,
    outer: UnknownFields,
    container: UnknownFields,
    wrapper: UnknownFields,
}

impl Envelope {
    /// Decode a `CloudStore` blob and return the envelope and its payload.
    pub(crate) fn decode(data: &[u8]) -> Result<(Self, &[u8]), BondError> {
        let mut reader = CompactBinaryReader::new(data);
        reader.read_marshaled_header()?;

        let mut outer = UnknownFields::default();
        let mut container = None;
        reader.read_fields(|reader, id, bond_type| match id {
            OUTER_CONTAINER => {
                expect_type(id, bond_type, BondType::Struct)?;
                container = Some(read_container(reader)?);
                Ok(())
            }
            _ => outer.capture(reader, id, bond_type),
        })?;
        reader.expect_end()?;

        let container = container.ok_or(BondError::MissingField(OUTER_CONTAINER))?;
        let envelope = Self {
            modified: container.modified,
            outer,
            container: container.unknown,
            wrapper: container.wrapper,
        };
        Ok((envelope, container.payload))
    }

    /// Encode `payload` inside this envelope.
    pub(crate) fn encode(&self, payload: &[u8]) -> Result<Vec<u8>, BondError> {
        let count = u32::try_from(payload.len())?;
        let mut writer = CompactBinaryWriter::new();
        writer.write_marshaled_header();
        writer.write_struct(&self.outer, |outer| {
            outer.field(OUTER_CONTAINER, BondType::Struct, |writer| {
                self.write_container(writer, count, payload);
            });
        });
        Ok(writer.into_bytes())
    }

    /// Return the last-modified time in Unix seconds.
    pub(crate) fn modified(&self) -> u64 {
        self.modified
    }

    /// Set the last-modified time for a write at `now`.
    ///
    /// Windows discards a write whose timestamp is older than the stored one
    /// and itself advances the timestamp by at least two seconds, so the new
    /// timestamp is never less than the previous one plus two seconds.
    pub(crate) fn stamp(&mut self, now: SystemTime) {
        self.modified = unix_secs_from_system_time(now)
            .max(self.modified.saturating_add(MIN_MODIFIED_STEP_SECS));
    }

    fn write_container(&self, writer: &mut CompactBinaryWriter, count: u32, payload: &[u8]) {
        writer.write_struct(&self.container, |container| {
            container.uint64(CONTAINER_MODIFIED, self.modified);
            container.field(CONTAINER_WRAPPER, BondType::Struct, |writer| {
                self.write_wrapper(writer, count, payload);
            });
        });
    }

    fn write_wrapper(&self, writer: &mut CompactBinaryWriter, count: u32, payload: &[u8]) {
        writer.write_struct(&self.wrapper, |wrapper| {
            wrapper.field(WRAPPER_PAYLOAD, BondType::List, |writer| {
                writer.write_list_header(BondType::Int8, count);
                writer.write_bytes(payload);
            });
        });
    }
}

struct Container<'a> {
    modified: u64,
    payload: &'a [u8],
    unknown: UnknownFields,
    wrapper: UnknownFields,
}

fn read_container<'a>(reader: &mut CompactBinaryReader<'a>) -> Result<Container<'a>, BondError> {
    let mut modified = None;
    let mut wrapper = None;
    let mut unknown = UnknownFields::default();
    reader.read_fields(|reader, id, bond_type| match id {
        CONTAINER_MODIFIED => {
            expect_type(id, bond_type, BondType::UInt64)?;
            modified = Some(reader.read_uint64()?);
            Ok(())
        }
        CONTAINER_WRAPPER => {
            expect_type(id, bond_type, BondType::Struct)?;
            wrapper = Some(read_wrapper(reader)?);
            Ok(())
        }
        _ => unknown.capture(reader, id, bond_type),
    })?;
    let (payload, wrapper) = wrapper.ok_or(BondError::MissingField(CONTAINER_WRAPPER))?;
    Ok(Container {
        modified: modified.ok_or(BondError::MissingField(CONTAINER_MODIFIED))?,
        payload,
        unknown,
        wrapper,
    })
}

fn read_wrapper<'a>(
    reader: &mut CompactBinaryReader<'a>,
) -> Result<(&'a [u8], UnknownFields), BondError> {
    let mut payload = None;
    let mut unknown = UnknownFields::default();
    reader.read_fields(|reader, id, bond_type| match id {
        WRAPPER_PAYLOAD => {
            expect_type(id, bond_type, BondType::List)?;
            let (element_type, count) = reader.read_list_header()?;
            expect_type(id, element_type, BondType::Int8)?;
            payload = Some(reader.read_bytes(usize::try_from(count)?)?);
            Ok(())
        }
        _ => unknown.capture(reader, id, bond_type),
    })?;
    let payload = payload.ok_or(BondError::MissingField(WRAPPER_PAYLOAD))?;
    Ok((payload, unknown))
}

/// Unwrap a `CloudStore` blob and return its timestamp and inner payload.
pub(crate) fn cloudstore_unwrap(data: &[u8]) -> Result<(u64, &[u8]), BondError> {
    let mut reader = CompactBinaryReader::new(data);
    reader.read_marshaled_header()?;

    let mut timestamp: Option<u64> = None;
    let mut payload: Option<&[u8]> = None;

    reader.read_fields(|reader, id, bond_type| match id {
        1 => {
            expect_type(id, bond_type, BondType::Struct)?;
            reader.read_fields(|reader, id, bond_type| match id {
                0 => {
                    expect_type(id, bond_type, BondType::UInt64)?;
                    timestamp = Some(reader.read_uint64()?);
                    Ok(())
                }
                1 => {
                    expect_type(id, bond_type, BondType::Struct)?;
                    reader.read_fields(|reader, id, bond_type| match id {
                        1 => {
                            expect_type(id, bond_type, BondType::List)?;
                            let (element_type, count) = reader.read_list_header()?;
                            expect_type(id, element_type, BondType::Int8)?;
                            payload = Some(reader.read_bytes(usize::try_from(count)?)?);
                            Ok(())
                        }
                        _ => reader.read_raw_value(bond_type).map(drop),
                    })
                }
                _ => reader.read_raw_value(bond_type).map(drop),
            })
        }
        _ => reader.read_raw_value(bond_type).map(drop),
    })?;
    reader.expect_end()?;

    let ts = timestamp.ok_or(BondError::MissingField(0))?;
    let bytes = payload.ok_or(BondError::MissingField(1))?;
    Ok((ts, bytes))
}

/// Wrap an inner payload into a `CloudStore` blob with `timestamp`.
pub(crate) fn cloudstore_wrap(timestamp: u64, inner_payload: &[u8]) -> Result<Vec<u8>, BondError> {
    let payload_len = u32::try_from(inner_payload.len())?;
    let none = UnknownFields::default();

    let mut writer = CompactBinaryWriter::new();
    writer.write_marshaled_header();
    writer.write_struct(&none, |outer| {
        outer.field(0, BondType::Struct, |w| {
            w.write_struct(&none, |meta| {
                meta.field(0, BondType::Bool, |w| w.write_bool(true));
            });
        });
        outer.field(1, BondType::Struct, |w| {
            w.write_struct(&none, |container| {
                container.field(0, BondType::UInt64, |w| w.write_uint64(timestamp));
                container.field(1, BondType::Struct, |w| {
                    w.write_struct(&none, |wrapper| {
                        wrapper.field(1, BondType::List, |w| {
                            w.write_list_header(BondType::Int8, payload_len);
                            w.write_bytes(inner_payload);
                        });
                    });
                });
            });
        });
    });
    Ok(writer.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use std::time::Duration;
    use std::time::UNIX_EPOCH;
    use test_case::test_case;

    #[test]
    fn decodes_modified_time_and_payload() {
        let (envelope, payload) =
            Envelope::decode(fixtures::SETTINGS_SET_HOURS).expect("fixture decodes");
        assert_eq!(envelope.modified(), 1_742_540_908);
        assert_eq!(payload.len(), 38);
        assert_eq!(payload.get(..4), Some(&[0x43, 0x42, 0x01, 0x00][..]));
    }

    #[test_case(fixtures::SETTINGS_SET_HOURS; "settings")]
    #[test_case(fixtures::STATE_LIVE; "state")]
    #[test_case(fixtures::SETTINGS_WITH_UNKNOWN_FIELDS; "unknown fields at every level")]
    fn round_trips_envelope(data: &[u8]) {
        let (envelope, payload) = Envelope::decode(data).expect("fixture decodes");
        assert_eq!(envelope.encode(payload).expect("envelope encodes"), data);
    }

    #[test]
    fn rejects_trailing_data() {
        let mut data = fixtures::STATE_LIVE.to_vec();
        data.push(0x00);
        let result = Envelope::decode(&data);
        assert!(
            matches!(result, Err(BondError::TrailingData(_))),
            "{result:?}"
        );
    }

    #[test_case(1_000, 2_000, 2_000; "clock ahead of stored time")]
    #[test_case(1_000, 1_001, 1_002; "clock within two seconds")]
    #[test_case(1_000, 500, 1_002; "clock behind stored time")]
    #[test_case(u64::MAX, 0, u64::MAX; "saturates")]
    fn stamps_modified_time(previous: u64, now: u64, expected: u64) {
        let (mut envelope, _) = Envelope::decode(fixtures::STATE_LIVE).expect("fixture decodes");
        envelope.modified = previous;
        envelope.stamp(UNIX_EPOCH + Duration::from_secs(now));
        assert_eq!(envelope.modified(), expected);
    }

    // The settings test bytes from nightlight_settings.rs
    const SETTINGS_BYTES: [u8; 60] = [
        0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0xEC, 0xA0, 0xF4, 0xBE, 0x06,
        0x2A, 0x2B, 0x0E, 0x26, 0x43, 0x42, 0x01, 0x00, 0x02, 0x01, 0xC2, 0x0A, 0x00, 0xCA, 0x14,
        0x0E, 0x01, 0x2E, 0x0F, 0x00, 0xCA, 0x1E, 0x00, 0xCF, 0x28, 0xCC, 0x2B, 0xCA, 0x32, 0x0E,
        0x13, 0x2E, 0x17, 0x00, 0xCA, 0x3C, 0x0E, 0x07, 0x2E, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    // The state (enabled) test bytes from nightlight_state.rs
    const STATE_ENABLED_BYTES: [u8; 43] = [
        0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0x89, 0x95, 0xFC, 0xBE, 0x06,
        0x2A, 0x2B, 0x0E, 0x15, 0x43, 0x42, 0x01, 0x00, 0x10, 0x00, 0xD0, 0x0A, 0x02, 0xC6, 0x14,
        0xA9, 0xF6, 0xE2, 0xD3, 0xEF, 0xEA, 0xE6, 0xED, 0x01, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn unwrap_settings() {
        let (timestamp, inner) =
            cloudstore_unwrap(&SETTINGS_BYTES).expect("settings fixture unwraps");
        assert_eq!(timestamp, 1_742_540_908);
        // Inner payload should start with CB header
        assert_eq!(&inner[..4], &[0x43, 0x42, 0x01, 0x00]);
        // Inner payload length = 38 (from list count 0x26)
        assert_eq!(inner.len(), 38);
    }

    #[test]
    fn unwrap_state_enabled() {
        let (timestamp, inner) =
            cloudstore_unwrap(&STATE_ENABLED_BYTES).expect("state fixture unwraps");
        assert_eq!(timestamp, 1_742_670_473);
        assert_eq!(&inner[..4], &[0x43, 0x42, 0x01, 0x00]);
        assert_eq!(inner.len(), 21);
    }

    #[test]
    fn wrap_roundtrip_settings() {
        let (timestamp, inner) =
            cloudstore_unwrap(&SETTINGS_BYTES).expect("settings fixture unwraps");
        let rewrapped = cloudstore_wrap(timestamp, inner).expect("payload wraps");
        assert_eq!(rewrapped, SETTINGS_BYTES);
    }

    #[test]
    fn wrap_roundtrip_state_enabled() {
        let (timestamp, inner) =
            cloudstore_unwrap(&STATE_ENABLED_BYTES).expect("state fixture unwraps");
        let rewrapped = cloudstore_wrap(timestamp, inner).expect("payload wraps");
        assert_eq!(rewrapped, STATE_ENABLED_BYTES);
    }
}
