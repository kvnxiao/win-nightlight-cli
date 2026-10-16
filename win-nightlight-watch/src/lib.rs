//! Observe Night Light changes and read decoded snapshots.
//!
//! Create one [`Watcher`] for settings and one for state as needed. Read each
//! initial snapshot with [`Watcher::read_current`] after constructing it.
//! Successful notification consumption rearms before reading. Snapshots reflect
//! the later read, and notifications can combine writes or produce duplicates.
//!
//! The watcher does not start a thread or write registry values. This crate
//! supports Windows only. Fallible operations return an [`Error`]; public
//! operations do not panic.
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//! use win_nightlight_watch::Snapshot;
//! use win_nightlight_watch::Watcher;
//!
//! fn observe(mut changed: impl FnMut(Snapshot)) -> Result<(), Box<dyn std::error::Error>> {
//!     let watcher = Watcher::settings()?;
//!     changed(watcher.read_current()?);
//!
//!     loop {
//!         if let Some(snapshot) = watcher.wait_timeout(Duration::from_secs(60))? {
//!             changed(snapshot);
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
use std::fmt;
use std::os::windows::io::AsHandle;
use std::os::windows::io::AsRawHandle;
use std::os::windows::io::BorrowedHandle;
use std::os::windows::io::FromRawHandle;
use std::os::windows::io::OwnedHandle;
use std::os::windows::io::RawHandle;
use std::ptr;
use std::time::Duration;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::SETTINGS_KEY_PATH;
use win_nightlight_lib::STATE_KEY_PATH;
use win_nightlight_lib::Settings;
use win_nightlight_lib::State;
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
use windows_sys::Win32::System::Threading::ResetEvent;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

const MAX_TIMEOUT_MILLIS: u32 = INFINITE - 1;

/// Store a decoded observation of one Night Light registry value.
///
/// The value reflects the read, not necessarily the write that signaled the
/// watcher. Settings and state are observed independently.
#[derive(Debug, Clone)]
pub enum Snapshot {
    /// Settings read by a watcher created with [`Watcher::settings`].
    Settings(Settings),
    /// State read by a watcher created with [`Watcher::state`].
    State(State),
}

/// Observe one Night Light record without a background thread.
///
/// - Use one notification consumer. `Watcher` is `Send` but not `Sync`.
/// - External waits observe readiness without consuming it. Then call
///   [`read_pending`](Self::read_pending) or
///   [`discard_pending`](Self::discard_pending).
/// - Do not reset or signal the exposed event yourself.
/// - Drop the watcher after a notification failure. An [`ErrorKind::Read`]
///   failure preserves registration and permits another read.
pub struct Watcher {
    // Closing the key can signal the event, so the key must drop first.
    key: Key,
    event: OwnedHandle,
    read: Box<dyn Fn() -> win_nightlight_lib::Result<Snapshot> + Send>,
}

impl Watcher {
    /// Register for Night Light settings changes without reading an initial
    /// snapshot.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`] if the settings key does not exist.
    /// - [`ErrorKind::KeyDeleted`] if Windows reports a deleted key.
    /// - [`ErrorKind::Windows`] if opening, event creation, or registration
    ///   fails.
    pub fn settings() -> Result<Self> {
        let nightlight = Nightlight::new();
        Self::open(SETTINGS_KEY_PATH, move || {
            nightlight.read_settings().map(Snapshot::Settings)
        })
    }

    /// Register for Night Light state changes without reading an initial
    /// snapshot.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`] if the state key does not exist.
    /// - [`ErrorKind::KeyDeleted`] if Windows reports a deleted key.
    /// - [`ErrorKind::Windows`] if opening, event creation, or registration
    ///   fails.
    pub fn state() -> Result<Self> {
        let nightlight = Nightlight::new();
        Self::open(STATE_KEY_PATH, move || {
            nightlight.read_state().map(Snapshot::State)
        })
    }

    /// Read the current record without consuming notifications or changing
    /// registration.
    ///
    /// Use for the initial snapshot or an independently scheduled retry.
    ///
    /// # Errors
    ///
    /// Return [`ErrorKind::Read`] with the underlying library error if reading
    /// or decoding fails. The watcher remains registered.
    pub fn read_current(&self) -> Result<Snapshot> {
        (self.read)().map_err(|source| Repr::Read { source }.into())
    }

    /// Wait for a notification, rearm, and return a freshly read snapshot.
    ///
    /// - Return `None` on timeout, without reading or changing registration.
    /// - The duration bounds the native wait, not the subsequent snapshot read.
    /// - Zero polls; positive fractional milliseconds round up. Time spent
    ///   asleep does not count toward the wait timeout on Windows 11.
    /// - Message-processing threads should use an external message-aware wait
    ///   followed by [`read_pending`](Self::read_pending).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidInput`] above 4,294,967,294 milliseconds, without
    ///   consuming a notification.
    /// - [`ErrorKind::Read`] if the snapshot read fails after successful
    ///   rearming.
    /// - Drop the watcher after any other error; notification handling failed.
    pub fn wait_timeout(&self, duration: Duration) -> Result<Option<Snapshot>> {
        if self.consume_notification(duration)? {
            self.read_current().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Consume a pending notification, rearm, and read a snapshot without
    /// waiting for a change.
    ///
    /// Return `None` without reading or rearming if no notification is pending.
    /// Use after an external wait on the watcher event.
    ///
    /// # Errors
    ///
    /// Return the errors of [`wait_timeout`](Self::wait_timeout).
    pub fn read_pending(&self) -> Result<Option<Snapshot>> {
        self.wait_timeout(Duration::ZERO)
    }

    /// Consume a pending notification and rearm without reading a snapshot.
    ///
    /// Return whether a notification was consumed. Use when snapshot reads
    /// must be deferred; call [`read_current`](Self::read_current) to retry
    /// later.
    ///
    /// # Errors
    ///
    /// Drop the watcher after any error; waiting, resetting, or registration
    /// failed.
    pub fn discard_pending(&self) -> Result<bool> {
        self.consume_notification(Duration::ZERO)
    }

    #[expect(unsafe_code, reason = "create and take ownership of a Win32 event")]
    fn open(
        path: &str,
        read: impl Fn() -> win_nightlight_lib::Result<Snapshot> + Send + 'static,
    ) -> Result<Self> {
        let key = CURRENT_USER
            .options()
            .access(KEY_NOTIFY)
            .open(path)
            .map_err(|source| Repr::Open { source })?;
        // SAFETY: Null attributes and name request an unnamed, non-inheritable
        // event. Manual reset keeps external waits from consuming readiness.
        let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if event.is_null() {
            return Err(Repr::CreateEvent {
                source: windows_result::Error::from_thread(),
            }
            .into());
        }
        // SAFETY: CreateEventW returned a valid, uniquely owned handle closed
        // by CloseHandle. Ownership transfers here exactly once.
        let event = unsafe { OwnedHandle::from_raw_handle(event) };
        let watcher = Self {
            key,
            event,
            read: Box::new(read),
        };
        watcher.rearm()?;
        Ok(watcher)
    }

    #[expect(unsafe_code, reason = "register an owned key with its owned event")]
    fn rearm(&self) -> Result<()> {
        // SAFETY: Both handles remain open throughout registration. The key
        // has KEY_NOTIFY access; construction or consumption precedes this
        // call.
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

    #[expect(unsafe_code, reason = "wait on and reset the watcher-owned event")]
    fn consume_notification(&self, duration: Duration) -> Result<bool> {
        let milliseconds = timeout_millis(duration)?;
        // SAFETY: The watcher keeps its event open throughout this wait.
        let status = unsafe { WaitForSingleObject(self.event.as_raw_handle(), milliseconds) };
        match status {
            WAIT_OBJECT_0 => {
                // SAFETY: The event is valid and has one consumer. Reset before
                // rearming so a subsequent notification cannot be erased.
                if unsafe { ResetEvent(self.event.as_raw_handle()) } == 0 {
                    return Err(Repr::ResetEvent {
                        source: windows_result::Error::from_thread(),
                    }
                    .into());
                }
                self.rearm()?;
                Ok(true)
            }
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(Repr::Wait {
                source: windows_result::Error::from_thread(),
            }
            .into()),
            other => Err(Repr::WaitStatus(other).into()),
        }
    }
}

impl fmt::Debug for Watcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Watcher")
            .field("key", &self.key)
            .field("event", &self.event)
            .finish_non_exhaustive()
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
