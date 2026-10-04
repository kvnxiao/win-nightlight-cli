use crate::bond::BondError;
use crate::settings::ColorTemperature;
use crate::store::Key;
use windows_result::HRESULT;
use windows_result::WIN32_ERROR;

const FILE_NOT_FOUND: HRESULT = WIN32_ERROR(2).to_hresult();
const MORE_DATA: HRESULT = WIN32_ERROR(234).to_hresult();

/// Result of a Night Light operation.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Failure to read, decode, encode, or write Night Light data, or an invalid
/// input value.
///
/// Use [`Error::kind`] to classify the failure; the source chain has the
/// underlying Windows or decoding error.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] Repr);

impl Error {
    /// Return the category of this failure.
    pub fn kind(&self) -> ErrorKind {
        match &self.0 {
            Repr::Read { source, .. } | Repr::Write { source, .. } => {
                if source.code() == FILE_NOT_FOUND {
                    ErrorKind::NotFound
                } else {
                    ErrorKind::Registry
                }
            }
            Repr::Decode { .. } => ErrorKind::Decode,
            Repr::Encode { .. } => ErrorKind::Encode,
            Repr::ColorTemperatureRange(_)
            | Repr::ColorTemperatureSyntax(_)
            | Repr::TimeOfDayRange { .. }
            | Repr::TimeOfDaySyntax(_) => ErrorKind::InvalidInput,
        }
    }

    pub(crate) fn may_be_torn_read(&self) -> bool {
        match &self.0 {
            Repr::Read { source, .. } => source.code() == MORE_DATA,
            Repr::Decode { .. } => true,
            _ => false,
        }
    }
}

/// Category of an [`Error`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The Night Light registry key or value does not exist.
    NotFound,
    /// The registry rejected a read or write.
    Registry,
    /// A stored value is not a valid Night Light payload.
    Decode,
    /// A value does not fit the Night Light payload format.
    Encode,
    /// A caller-supplied value is out of range or malformed.
    InvalidInput,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Repr {
    #[error("reading {key} from the registry")]
    Read {
        key: Key,
        source: windows_result::Error,
    },
    #[error("writing {key} to the registry")]
    Write {
        key: Key,
        source: windows_result::Error,
    },
    #[error("decoding {key}")]
    Decode { key: Key, source: BondError },
    #[error("encoding {key}")]
    Encode { key: Key, source: BondError },
    #[error(
        "color temperature {0}K is outside {min}-{max}",
        min = ColorTemperature::MIN,
        max = ColorTemperature::MAX,
    )]
    ColorTemperatureRange(u32),
    #[error(
        "invalid color temperature {0:?}, expected kelvin from {min} to {max}",
        min = ColorTemperature::MIN.kelvin(),
        max = ColorTemperature::MAX.kelvin(),
    )]
    ColorTemperatureSyntax(String),
    #[error("time of day {hour}:{minute:02} is out of range")]
    TimeOfDayRange { hour: u8, minute: u8 },
    #[error("invalid time of day {0:?}, expected HH:MM")]
    TimeOfDaySyntax(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Error>();
    }

    #[test]
    fn missing_registry_value_is_not_found() {
        let error = Error::from(Repr::Read {
            key: Key::State,
            source: WIN32_ERROR(2).into(),
        });
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(
            error.to_string(),
            "reading Night Light state from the registry"
        );
        assert!(error.source().is_some());
    }

    #[test]
    fn denied_registry_write_is_registry_error() {
        let error = Error::from(Repr::Write {
            key: Key::Settings,
            source: WIN32_ERROR(5).into(),
        });
        assert_eq!(error.kind(), ErrorKind::Registry);
        assert_eq!(
            error.to_string(),
            "writing Night Light settings to the registry"
        );
    }
}
