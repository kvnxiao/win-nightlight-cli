use crate::MAX_TIMEOUT_MILLIS;
use windows_result::WIN32_ERROR;
use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_sys::Win32::Foundation::ERROR_KEY_DELETED;
use windows_sys::Win32::Foundation::ERROR_PATH_NOT_FOUND;

/// Result of a registry watcher operation.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Report a watcher failure and preserve its underlying Windows error.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] Repr);

impl Error {
    /// Return the category of this failure.
    pub fn kind(&self) -> ErrorKind {
        match &self.0 {
            Repr::InvalidPath | Repr::Timeout => ErrorKind::InvalidInput,
            Repr::WaitStatus(_) => ErrorKind::Windows,
            Repr::Open { source }
            | Repr::CreateEvent { source }
            | Repr::Register { source }
            | Repr::Wait { source } => match WIN32_ERROR::from_error(source).map(|code| code.0) {
                Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => ErrorKind::NotFound,
                Some(ERROR_KEY_DELETED) => ErrorKind::KeyDeleted,
                _ => ErrorKind::Windows,
            },
        }
    }
}

/// Classify a watcher failure.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The requested registry key does not exist.
    NotFound,
    /// Windows reports that the watched key was deleted.
    KeyDeleted,
    /// The path is empty or has a NUL, or the timeout exceeds the finite range.
    InvalidInput,
    /// A Windows registry or event operation failed.
    Windows,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Repr {
    #[error("registry path must be nonempty and must not contain NUL")]
    InvalidPath,
    #[error("timeout exceeds {MAX_TIMEOUT_MILLIS} milliseconds")]
    Timeout,
    #[error("opening the registry key for notifications")]
    Open { source: windows_result::Error },
    #[error("creating the notification event")]
    CreateEvent { source: windows_result::Error },
    #[error("registering for registry changes")]
    Register { source: windows_result::Error },
    #[error("waiting for a registry notification")]
    Wait { source: windows_result::Error },
    #[error("unexpected notification wait result {0:#x}")]
    WaitStatus(u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;
    use test_case::test_case;

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Error>();
    }

    #[test_case(2, ErrorKind::NotFound)]
    #[test_case(3, ErrorKind::NotFound)]
    #[test_case(1018, ErrorKind::KeyDeleted)]
    #[test_case(5, ErrorKind::Windows)]
    fn windows_errors_preserve_category_and_code(code: u32, expected: ErrorKind) {
        let error = Error::from(Repr::Register {
            source: WIN32_ERROR(code).into(),
        });
        assert_eq!(error.kind(), expected);
        assert_eq!(error.to_string(), "registering for registry changes");
        let source = error
            .source()
            .expect("Windows error is preserved")
            .downcast_ref::<windows_result::Error>()
            .expect("source is a Windows error");
        assert_eq!(WIN32_ERROR::from_error(source), Some(WIN32_ERROR(code)));
    }
}
