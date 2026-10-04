use crate::Result;
use crate::bond::BondError;
use crate::bond::BondType;
use crate::bond::CompactBinaryReader;
use crate::bond::CompactBinaryWriter;
use crate::bond::UnknownFields;
use crate::bond::expect_type;
use crate::cloudstore::Envelope;
use crate::error::Repr;
use crate::store::Key;
use crate::time::filetime_from_system_time;
use crate::time::system_time_from_filetime;
use crate::time::system_time_from_unix_secs;
use std::fmt;
use std::time::SystemTime;

const STATUS: u16 = 0;
const TRANSITION_CAUSE: u16 = 10;
const LAST_TRANSITION: u16 = 20;
const USABLE: u16 = 30;

const STATUS_RUNNING: i32 = 0;
const CAUSE_SCHEDULED: i32 = 0;
const CAUSE_MANUAL: i32 = 1;

/// What caused the last change between on and off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionCause {
    /// The schedule turned Night Light on or off.
    Scheduled,
    /// A person or program turned Night Light on or off.
    Manual,
}

impl fmt::Display for TransitionCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Scheduled => "scheduled",
            Self::Manual => "manual",
        })
    }
}

/// Night Light on/off state.
///
/// Windows writes this value both for manual changes and for scheduled
/// transitions. Decoding keeps fields this type does not model and encoding
/// writes them back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    envelope: Envelope,
    status: Option<i32>,
    transition_cause: i32,
    last_transition: u64,
    usable: bool,
    unknown: UnknownFields,
}

impl State {
    /// Decode state from the registry value bytes.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`ErrorKind::Decode`] if `data` is not a valid
    /// Night Light state payload.
    ///
    /// [`ErrorKind::Decode`]: crate::ErrorKind::Decode
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::decode(data).map_err(|source| {
            Repr::Decode {
                key: Key::State,
                source,
            }
            .into()
        })
    }

    /// Encode this state as registry value bytes, keeping the stored
    /// last-modified time.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`ErrorKind::Encode`] if the payload is too
    /// large for the format.
    ///
    /// [`ErrorKind::Encode`]: crate::ErrorKind::Encode
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.encode().map_err(|source| {
            Repr::Encode {
                key: Key::State,
                source,
            }
            .into()
        })
    }

    /// Return whether Night Light is currently on.
    pub fn is_active(&self) -> bool {
        self.status.is_some()
    }

    /// Return what caused the last change between on and off.
    pub fn transition_cause(&self) -> TransitionCause {
        if self.transition_cause == CAUSE_SCHEDULED {
            TransitionCause::Scheduled
        } else {
            TransitionCause::Manual
        }
    }

    /// Return when Night Light last turned on or off, or `None` if the time is
    /// unrecorded or out of range.
    pub fn last_transition(&self) -> Option<SystemTime> {
        (self.last_transition != 0)
            .then(|| system_time_from_filetime(self.last_transition))
            .flatten()
    }

    /// Return whether Windows reports Night Light as usable on this device.
    pub fn is_usable(&self) -> bool {
        self.usable
    }

    /// Return when the state was last written, or `None` if the stored time
    /// is out of range.
    pub fn modified(&self) -> Option<SystemTime> {
        system_time_from_unix_secs(self.envelope.modified())
    }

    /// Turn Night Light on or off.
    ///
    /// This does not record a transition; [`Nightlight::update_state`]
    /// records a manual transition when the active state changes.
    ///
    /// [`Nightlight::update_state`]: crate::Nightlight::update_state
    pub fn set_active(&mut self, active: bool) {
        self.status = if active {
            self.status.or(Some(STATUS_RUNNING))
        } else {
            None
        };
    }

    pub(crate) fn stamp(&mut self, now: SystemTime) {
        self.envelope.stamp(now);
    }

    pub(crate) fn record_manual_transition(&mut self, now: SystemTime) {
        self.transition_cause = CAUSE_MANUAL;
        self.last_transition = filetime_from_system_time(now);
    }

    fn decode(data: &[u8]) -> Result<Self, BondError> {
        let (envelope, payload) = Envelope::decode(data)?;
        let mut reader = CompactBinaryReader::new(payload);
        reader.read_marshaled_header()?;

        let mut status = None;
        let mut transition_cause = CAUSE_SCHEDULED;
        let mut last_transition = 0;
        let mut usable = true;
        let mut unknown = UnknownFields::default();
        reader.read_fields(|reader, id, bond_type| match id {
            STATUS => {
                expect_type(id, bond_type, BondType::Int32)?;
                status = Some(reader.read_int32()?);
                Ok(())
            }
            TRANSITION_CAUSE => {
                expect_type(id, bond_type, BondType::Int32)?;
                transition_cause = reader.read_int32()?;
                Ok(())
            }
            LAST_TRANSITION => {
                expect_type(id, bond_type, BondType::UInt64)?;
                last_transition = reader.read_uint64()?;
                Ok(())
            }
            USABLE => {
                expect_type(id, bond_type, BondType::Bool)?;
                usable = reader.read_bool()?;
                Ok(())
            }
            _ => unknown.capture(reader, id, bond_type),
        })?;
        reader.expect_end()?;

        Ok(Self {
            envelope,
            status,
            transition_cause,
            last_transition,
            usable,
            unknown,
        })
    }

    fn encode(&self) -> Result<Vec<u8>, BondError> {
        let mut writer = CompactBinaryWriter::new();
        writer.write_marshaled_header();
        writer.write_struct(&self.unknown, |fields| {
            if let Some(status) = self.status {
                fields.int32(STATUS, status);
            }
            if self.transition_cause != CAUSE_SCHEDULED {
                fields.int32(TRANSITION_CAUSE, self.transition_cause);
            }
            if self.last_transition != 0 {
                fields.uint64(LAST_TRANSITION, self.last_transition);
            }
            if !self.usable {
                fields.bool(USABLE, false);
            }
        });
        self.envelope.encode(&writer.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;
    use crate::fixtures;
    use proptest::prelude::*;
    use std::time::Duration;
    use std::time::UNIX_EPOCH;
    use test_case::test_case;

    fn decode(data: &[u8]) -> State {
        State::from_bytes(data).expect("fixture decodes")
    }

    #[test_case(fixtures::STATE_INACTIVE; "inactive")]
    #[test_case(fixtures::STATE_ACTIVE; "active")]
    #[test_case(fixtures::STATE_LIVE; "live")]
    #[test_case(fixtures::STATE_UNUSABLE; "unusable")]
    #[test_case(fixtures::STATE_WITH_UNKNOWN_FIELD; "unknown field")]
    fn round_trips_bytes(data: &[u8]) {
        assert_eq!(decode(data).to_bytes().expect("state encodes"), data);
    }

    #[test_case(fixtures::STATE_INACTIVE, false)]
    #[test_case(fixtures::STATE_ACTIVE, true)]
    fn decodes_original_fixtures(data: &[u8], active: bool) {
        let state = decode(data);
        assert_eq!(state.is_active(), active);
        assert_eq!(state.transition_cause(), TransitionCause::Manual);
        assert_eq!(
            state.last_transition(),
            Some(UNIX_EPOCH + Duration::new(1_742_667_580, 927_056_900))
        );
        assert!(state.is_usable());
        assert_eq!(
            state.modified(),
            Some(UNIX_EPOCH + Duration::from_secs(1_742_670_473))
        );
    }

    #[test]
    fn decodes_live_fixture() {
        let state = decode(fixtures::STATE_LIVE);
        assert!(!state.is_active());
        assert_eq!(state.transition_cause(), TransitionCause::Manual);
        assert_eq!(
            state.last_transition(),
            Some(UNIX_EPOCH + Duration::new(1_774_515_867, 967_889_600))
        );
        assert_eq!(
            state.modified(),
            Some(UNIX_EPOCH + Duration::from_secs(1_774_515_869))
        );
    }

    #[test]
    fn decodes_unusable_flag() {
        assert!(!decode(fixtures::STATE_UNUSABLE).is_usable());
    }

    #[test_case(fixtures::STATE_INACTIVE, true, fixtures::STATE_ACTIVE; "turn on")]
    #[test_case(fixtures::STATE_ACTIVE, false, fixtures::STATE_INACTIVE; "turn off")]
    #[test_case(fixtures::STATE_ACTIVE, true, fixtures::STATE_ACTIVE; "keep on")]
    #[test_case(fixtures::STATE_WITH_UNKNOWN_FIELD, true, fixtures::STATE_WITH_UNKNOWN_FIELD_ACTIVE; "turn on with unknown field")]
    fn set_active_changes_only_status(data: &[u8], active: bool, expected: &[u8]) {
        let mut state = decode(data);
        state.set_active(active);
        assert_eq!(state.to_bytes().expect("state encodes"), expected);
    }

    #[test]
    fn rejects_known_field_with_wrong_type() {
        let error = State::from_bytes(fixtures::STATE_STATUS_AS_BOOL)
            .expect_err("status with bool type is invalid");
        assert_eq!(error.kind(), ErrorKind::Decode);
        assert_eq!(error.to_string(), "decoding Night Light state");
    }

    proptest! {
        #[test]
        fn set_active_round_trips(active: bool) {
            let mut state = decode(fixtures::STATE_WITH_UNKNOWN_FIELD);
            state.set_active(active);
            let decoded = decode(&state.to_bytes().expect("state encodes"));
            prop_assert_eq!(decoded.is_active(), active);
            prop_assert_eq!(decoded, state);
        }
    }
}
