//! Wait for changes to existing registry keys under `HKEY_CURRENT_USER`.
//!
//! Use one consumer per watcher. Create the watcher before the initial read;
//! after a successful wait, rearm once before reading again. Notifications can
//! combine multiple writes. Do not rearm after a timeout. Drop the watcher
//! after any rearm error; it does not follow a deleted key's replacement.
//!
//! This crate supports Windows only. Fallible operations return an [`Error`];
//! public operations do not panic. The watcher does not read or write values.
//!
//! # Example
//!
//! Observe Night Light state with `win-nightlight-lib`:
//!
//! ```no_run
//! use std::time::Duration;
//! use win_nightlight_lib::Nightlight;
//! use win_nightlight_lib::STATE_KEY_PATH;
//! use win_nightlight_lib::State;
//! use win_nightlight_watch::Watcher;
//!
//! fn observe(mut changed: impl FnMut(State)) -> Result<(), Box<dyn std::error::Error>> {
//!     let watcher = Watcher::new(STATE_KEY_PATH)?;
//!     let nightlight = Nightlight::new();
//!     changed(nightlight.read_state()?);
//!
//!     loop {
//!         if watcher.wait_timeout(Duration::from_secs(60))? {
//!             watcher.rearm()?;
//!             changed(nightlight.read_state()?);
//!         }
//!     }
//! }
//! ```

#![warn(missing_debug_implementations)]

mod error;
#[cfg(test)]
mod tests;

pub use crate::error::Error;
pub use crate::error::ErrorKind;
use crate::error::Repr;
pub use crate::error::Result;
use std::os::windows::io::AsHandle;
use std::os::windows::io::AsRawHandle;
use std::os::windows::io::BorrowedHandle;
use std::os::windows::io::FromRawHandle;
use std::os::windows::io::OwnedHandle;
use std::os::windows::io::RawHandle;
use std::ptr;
use std::time::Duration;
use windows_registry::CURRENT_USER;
use windows_registry::Key;
use windows_result::WIN32_ERROR;
use windows_sys::Win32::Foundation::WAIT_FAILED;
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
use windows_sys::Win32::System::Registry::KEY_NOTIFY;
use windows_sys::Win32::System::Registry::REG_NOTIFY_CHANGE_LAST_SET;
use windows_sys::Win32::System::Registry::REG_NOTIFY_THREAD_AGNOSTIC;
use windows_sys::Win32::System::Registry::RegNotifyChangeKeyValue;
use windows_sys::Win32::System::Threading::CreateEventW;
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

const MAX_TIMEOUT_MILLIS: u32 = INFINITE - 1;

/// Watch value additions, modifications, and deletions in one registry key.
///
/// - Use one notification consumer, including waits on the exposed event
///   handle.
/// - After consuming a notification, rearm once before reading current values.
/// - Drop the watcher after any rearm error.
/// - `Watcher` is `Send` but not `Sync`.
#[derive(Debug)]
pub struct Watcher {
    // Closing the key can signal the event, so the key must drop first.
    key: Key,
    event: OwnedHandle,
}

impl Watcher {
    /// Register for changes to an existing subkey of `HKEY_CURRENT_USER`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidInput`] for an empty path or a NUL character.
    /// - [`ErrorKind::NotFound`] if the key does not exist.
    /// - [`ErrorKind::KeyDeleted`] if Windows reports a deleted key.
    /// - [`ErrorKind::Windows`] if opening, event creation, or registration
    ///   fails.
    #[expect(unsafe_code, reason = "create and take ownership of a Win32 event")]
    pub fn new(path: &str) -> Result<Self> {
        if path.is_empty() || path.contains('\0') {
            return Err(Repr::InvalidPath.into());
        }

        let key = CURRENT_USER
            .options()
            .access(KEY_NOTIFY)
            .open(path)
            .map_err(|source| Repr::Open { source })?;
        // SAFETY: Null attributes and name request an unnamed, non-inheritable
        // event; the remaining arguments select auto-reset and nonsignaled.
        let event = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
        if event.is_null() {
            return Err(Repr::CreateEvent {
                source: windows_result::Error::from_thread(),
            }
            .into());
        }
        // SAFETY: CreateEventW returned a valid, uniquely owned handle closed
        // by CloseHandle. Ownership transfers here exactly once.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        let watcher = Self { key, event };
        watcher.rearm()?;
        Ok(watcher)
    }

    /// Register for the next notification after a successful wait.
    ///
    /// Call exactly once after consuming the previous notification and before
    /// reading values. Calling while a registration is pending leaks native
    /// wait operations. Do not call after a timeout.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::KeyDeleted`] if the watched key was deleted.
    /// - [`ErrorKind::Windows`] for another registration failure.
    /// - Drop this watcher after any error; do not wait or rearm again.
    #[expect(unsafe_code, reason = "register an owned key with its owned event")]
    pub fn rearm(&self) -> Result<()> {
        // SAFETY: Both handles remain open throughout the call and
        // registration. The key has KEY_NOTIFY access, and drop closes
        // it before the event.
        let status = unsafe {
            RegNotifyChangeKeyValue(
                self.key.as_raw(),
                0,
                REG_NOTIFY_CHANGE_LAST_SET | REG_NOTIFY_THREAD_AGNOSTIC,
                self.event.as_raw_handle(),
                1,
            )
        };
        WIN32_ERROR(status)
            .ok()
            .map_err(|source| Repr::Register { source }.into())
    }

    /// Consume a notification or return `false` when the timeout expires.
    ///
    /// Zero polls immediately. Positive fractional milliseconds round up.
    /// Time spent asleep does not count toward the timeout. A timeout keeps
    /// the current registration; a successful wait requires rearming.
    ///
    /// Threads that process window messages should use a message-aware wait
    /// on [`Self::as_handle`] instead of blocking here.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidInput`] above 4,294,967,294 milliseconds.
    /// - [`ErrorKind::Windows`] if the wait fails.
    #[expect(unsafe_code, reason = "wait on the event owned by this watcher")]
    pub fn wait_timeout(&self, duration: Duration) -> Result<bool> {
        let milliseconds = timeout_millis(duration)?;
        // SAFETY: The borrowed watcher keeps this event open for the entire
        // wait.
        let status = unsafe { WaitForSingleObject(self.event.as_raw_handle(), milliseconds) };
        match status {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(Repr::Wait {
                source: windows_result::Error::from_thread(),
            }
            .into()),
            other => Err(Repr::WaitStatus(other).into()),
        }
    }
}

impl AsHandle for Watcher {
    fn as_handle(&self) -> BorrowedHandle<'_> {
        self.event.as_handle()
    }
}

impl AsRawHandle for Watcher {
    fn as_raw_handle(&self) -> RawHandle {
        self.event.as_raw_handle()
    }
}

fn timeout_millis(duration: Duration) -> Result<u32> {
    let milliseconds =
        u32::try_from(duration.as_nanos().div_ceil(1_000_000)).map_err(|_error| Repr::Timeout)?;
    if milliseconds > MAX_TIMEOUT_MILLIS {
        return Err(Repr::Timeout.into());
    }
    Ok(milliseconds)
}
