//! Byte layouts of the two `CloudStore` records Windows writes for Night Light
//! settings, and the fields the probe changes in them.
//!
//! The probe patches these bytes directly because `win-nightlight-lib` only
//! writes timestamps that move forward and does not set the preview flag on
//! its own.

use anyhow::Context as _;
use anyhow::bail;
use anyhow::ensure;

// Marshaled header, metadata struct holding `true`, then the payload container
// whose first field is the `uint64` timestamp.
const SETTINGS_PREFIX: [u8; 10] = [0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06];
// The data wrapper struct, then its field 1 as a `list<int8>` whose count
// follows. Windows writes the wrapper without other fields.
const PAYLOAD_LIST: [u8; 3] = [0x2A, 0x2B, 0x0E];
// Field 70, `previewing`, as `bool` `true`. It is the highest field ID, so it
// is the last field before the payload's closing `BT_STOP`.
const PREVIEW_FIELD: [u8; 3] = [0xC2, 0x46, 0x01];
// Marshaled header, empty metadata struct, then field 1 as the `uint64`
// timestamp. The record has no payload.
const CLOUD_PREFIX: [u8; 7] = [0x43, 0x42, 0x01, 0x00, 0x0A, 0x00, 0x26];
const STOP: u8 = 0x00;
// Closes the data wrapper, the payload container, and the outer struct.
const ENVELOPE_END: [u8; 3] = [STOP; 3];
const VARINT_CONTINUE: u8 = 0x80;
const VARINT_PAYLOAD: u8 = 0x7F;
const VARINT_BITS: u32 = 7;

pub(crate) fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let [low, ..] = (value & u64::from(VARINT_PAYLOAD)).to_le_bytes();
        value >>= VARINT_BITS;
        if value == 0 {
            bytes.push(low);
            return bytes;
        }
        bytes.push(low | VARINT_CONTINUE);
    }
}

fn decode_varint(bytes: &[u8]) -> anyhow::Result<(u64, usize)> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    for (index, &byte) in bytes.iter().enumerate() {
        let payload = u64::from(byte & VARINT_PAYLOAD);
        ensure!(
            shift < u64::BITS && payload.leading_zeros() >= shift,
            "varint does not fit in 64 bits"
        );
        value |= payload << shift;
        if byte & VARINT_CONTINUE == 0 {
            return Ok((value, index + 1));
        }
        shift += VARINT_BITS;
    }
    bail!("varint is unterminated")
}

/// Decode the timestamp of the latest Windows Settings write from a `Cloud`
/// settings record.
pub(crate) fn cloud_modified(data: &[u8]) -> anyhow::Result<u64> {
    let rest = data
        .strip_prefix(&CLOUD_PREFIX[..])
        .context("unexpected Cloud record layout")?;
    let (modified, len) = decode_varint(rest)?;
    ensure!(
        rest.get(len..) == Some(&[STOP][..]),
        "unexpected data after the Cloud timestamp"
    );
    Ok(modified)
}

pub(crate) fn cloud_record(modified: u64) -> Vec<u8> {
    let mut data = CLOUD_PREFIX.to_vec();
    data.extend(encode_varint(modified));
    data.push(STOP);
    data
}

/// A `Current` settings value split into its timestamp and its inner
/// marshaled payload.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SettingsEnvelope {
    pub(crate) modified: u64,
    payload: Vec<u8>,
}

impl SettingsEnvelope {
    /// Split a settings value in the layout Windows writes.
    ///
    /// # Errors
    ///
    /// Returns an error for any other layout, including one whose data
    /// wrapper has fields besides the payload.
    pub(crate) fn parse(data: &[u8]) -> anyhow::Result<Self> {
        let rest = data
            .strip_prefix(&SETTINGS_PREFIX[..])
            .context("unexpected settings envelope layout")?;
        let (modified, len) = decode_varint(rest)?;
        let rest = rest
            .get(len..)
            .and_then(|rest| rest.strip_prefix(&PAYLOAD_LIST[..]))
            .context("unexpected settings data wrapper layout")?;
        let (count, len) = decode_varint(rest)?;
        let count = usize::try_from(count)?;
        let payload = rest
            .get(len..)
            .and_then(|rest| rest.get(..count))
            .context("settings payload is shorter than its count")?;
        ensure!(
            rest.get(len + count..) == Some(&ENVELOPE_END[..]),
            "unexpected data after the settings payload"
        );
        Ok(Self {
            modified,
            payload: payload.to_vec(),
        })
    }

    pub(crate) fn encode(&self) -> anyhow::Result<Vec<u8>> {
        let mut data = SETTINGS_PREFIX.to_vec();
        data.extend(encode_varint(self.modified));
        data.extend_from_slice(&PAYLOAD_LIST);
        data.extend(encode_varint(u64::try_from(self.payload.len())?));
        data.extend_from_slice(&self.payload);
        data.extend_from_slice(&ENVELOPE_END);
        Ok(data)
    }

    /// Set or clear the payload's preview flag, keeping every other field.
    pub(crate) fn set_previewing(&mut self, previewing: bool) -> anyhow::Result<()> {
        let fields = self
            .payload
            .strip_suffix(&[STOP])
            .context("settings payload does not end its struct")?;
        let mut payload = fields
            .strip_suffix(&PREVIEW_FIELD[..])
            .unwrap_or(fields)
            .to_vec();
        if previewing {
            payload.extend_from_slice(&PREVIEW_FIELD);
        }
        payload.push(STOP);
        self.payload = payload;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;
    use test_case::test_case;
    use win_nightlight_lib::Settings;

    const SETTINGS: &[u8] = &hex!(
        "43420100 0A020100 2A 06EFD487D606 2A 2B0E21"
        "43420100 CA140E1500 CA1E0E0700 CF28C435 CA320E132E1100 CA3C0E062E2600 00"
        "00 00 00"
    );
    const SETTINGS_PREVIEWING: &[u8] = &hex!(
        "43420100 0A020100 2A 06EFD487D606 2A 2B0E24"
        "43420100 CA140E1500 CA1E0E0700 CF28C435 CA320E132E1100 CA3C0E062E2600 C24601 00"
        "00 00 00"
    );

    #[test_case(0, &[0x00]; "zero")]
    #[test_case(127, &[0x7F]; "largest single byte")]
    #[test_case(128, &[0x80, 0x01]; "smallest two bytes")]
    #[test_case(1_792_364_735, &hex!("BFA1D5D606"); "timestamp written by Windows")]
    #[test_case(u64::MAX, &hex!("FFFFFFFFFFFFFFFFFF01"); "maximum")]
    fn encodes_varint(value: u64, expected: &[u8]) {
        assert_eq!(encode_varint(value), expected);
    }

    #[test]
    fn decodes_cloud_record_written_by_windows() {
        let modified = cloud_modified(&hex!("43420100 0A00 26 C5A7D5D606 00"))
            .expect("record written by explorer.exe decodes");
        assert_eq!(modified, 1_792_365_509);
    }

    #[test]
    fn encodes_cloud_record_like_windows() {
        assert_eq!(
            cloud_record(1_792_365_509),
            hex!("43420100 0A00 26 C5A7D5D606 00")
        );
    }

    #[test_case(SETTINGS; "current settings envelope")]
    #[test_case(&hex!("43420100 0A00 26 C5A7D5D606 00 00"); "trailing data")]
    #[test_case(&hex!("43420100 0A00 26 C5A7"); "unterminated timestamp")]
    #[test_case(&hex!("43420100 0A00 26 FFFFFFFFFFFFFFFFFF7F 00"); "timestamp overflows")]
    fn rejects_invalid_cloud_record(data: &[u8]) {
        let result = cloud_modified(data);
        assert!(result.is_err(), "{result:?}");
    }

    #[test_case(SETTINGS; "preview off")]
    #[test_case(SETTINGS_PREVIEWING; "preview on")]
    fn round_trips_settings_written_by_windows(data: &[u8]) {
        let envelope = SettingsEnvelope::parse(data).expect("settings envelope parses");
        assert_eq!(envelope.modified, 1_791_093_359);
        assert_eq!(envelope.encode().expect("settings envelope encodes"), data);
    }

    #[test_case(1_792_365_509, &hex!("C5A7D5D606"); "equal length")]
    #[test_case(0, &hex!("00"); "shorter")]
    fn replaces_settings_timestamp(modified: u64, encoded: &[u8]) {
        let mut envelope = SettingsEnvelope::parse(SETTINGS).expect("settings envelope parses");
        envelope.modified = modified;
        let mut expected = SETTINGS_PREFIX.to_vec();
        expected.extend_from_slice(encoded);
        expected.extend_from_slice(SETTINGS.get(15..).expect("fixture has a payload"));
        assert_eq!(
            envelope.encode().expect("settings envelope encodes"),
            expected
        );
    }

    #[test_case(SETTINGS, true, SETTINGS_PREVIEWING; "sets preview")]
    #[test_case(SETTINGS_PREVIEWING, false, SETTINGS; "clears preview")]
    #[test_case(SETTINGS_PREVIEWING, true, SETTINGS_PREVIEWING; "keeps preview set")]
    #[test_case(SETTINGS, false, SETTINGS; "keeps preview clear")]
    fn changes_preview_flag(data: &[u8], previewing: bool, expected: &[u8]) {
        let mut envelope = SettingsEnvelope::parse(data).expect("settings envelope parses");
        envelope
            .set_previewing(previewing)
            .expect("preview flag changes");
        let encoded = envelope.encode().expect("settings envelope encodes");
        assert_eq!(encoded, expected);
        let decoded = Settings::from_bytes(&encoded).expect("library decodes the patched value");
        assert_eq!(decoded.is_previewing(), previewing);
        assert_eq!(decoded.color_temperature(), Some(3426));
    }

    #[test_case(&cloud_record(1); "Cloud record")]
    #[test_case(&hex!("43420100 0A020100 2A 06EFD487D606 2A 0605 2B0E03 430000 00 00 00"); "wrapper with another field")]
    #[test_case(&hex!("43420100 0A020100 2A 06EFD487D606 2A 2B0E05 4342 00 00 00"); "payload shorter than its count")]
    #[test_case(&hex!("43420100 0A020100 2A 06EFD487D606 2A 2B0E01 00 00 00 00 00"); "trailing data")]
    fn rejects_unexpected_settings_layout(data: &[u8]) {
        let result = SettingsEnvelope::parse(data);
        assert!(result.is_err(), "{result:?}");
    }
}
