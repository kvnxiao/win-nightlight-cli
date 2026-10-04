use crate::bond::BondError;
use crate::bond::BondType;
use crate::bond::CompactBinaryReader;
use crate::bond::CompactBinaryWriter;
use crate::bond::UnknownFields;
use crate::bond::expect_type;

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
