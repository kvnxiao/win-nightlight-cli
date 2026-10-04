//! Read and change Windows 11 Night Light in the current user's registry.
//!
//! Windows stores Night Light in two registry values:
//!
//! - [`Settings`]: the schedule and the color temperature.
//! - [`State`]: whether Night Light is on now, and when and why it last turned
//!   on or off.
//!
//! [`Nightlight`] reads and writes each value on its own, so turning Night
//! Light on or off does not change the schedule, and changing a setting does
//! not turn Night Light on or off.
//!
//! Both values are Bond `CompactBinary` v1 payloads inside a `CloudStore`
//! envelope. Microsoft does not document this format; this crate follows
//! reverse-engineered schemas. Decoding keeps fields this crate does not
//! model, except fields inside a time of day, and encoding writes them back
//! unchanged.
//!
//! This crate supports Windows only. No public function panics; every
//! fallible operation returns an [`Error`] that [`Error::kind`] classifies.
//!
//! # Examples
//!
//! Turn Night Light on until the schedule turns it off:
//!
//! ```no_run
//! use win_nightlight_lib::Nightlight;
//!
//! Nightlight::new().set_active(true)?;
//! # Ok::<(), win_nightlight_lib::Error>(())
//! ```
//!
//! Set the color temperature and a set-hours schedule in one write:
//!
//! ```no_run
//! use win_nightlight_lib::ColorTemperature;
//! use win_nightlight_lib::Nightlight;
//! use win_nightlight_lib::Schedule;
//! use win_nightlight_lib::TimeOfDay;
//!
//! let temperature: ColorTemperature = "3400K".parse()?;
//! let start: TimeOfDay = "21:00".parse()?;
//! let end: TimeOfDay = "07:00".parse()?;
//!
//! Nightlight::new().update_settings(|settings| {
//!     settings.set_color_temperature(temperature);
//!     settings.set_schedule(Schedule::SetHours {
//!         start: Some(start),
//!         end: Some(end),
//!     });
//! })?;
//! # Ok::<(), win_nightlight_lib::Error>(())
//! ```
//!
//! Decoding rejects bytes that are not a Night Light settings payload:
//!
//! ```
//! use win_nightlight_lib::ErrorKind;
//! use win_nightlight_lib::Settings;
//!
//! let result = Settings::from_bytes(&[0x43, 0x42, 0x01, 0x00]);
//! assert!(result.is_err_and(|error| error.kind() == ErrorKind::Decode));
//! ```

mod bond;
mod cloudstore;
mod error;
#[cfg(test)]
mod fixtures;
mod nightlight;
mod settings;
mod state;
mod store;
mod time;

pub use crate::error::Error;
pub use crate::error::ErrorKind;
pub use crate::error::Result;
pub use crate::nightlight::Nightlight;
pub use crate::settings::ColorTemperature;
pub use crate::settings::Schedule;
pub use crate::settings::ScheduleKind;
pub use crate::settings::ScheduleMode;
pub use crate::settings::Settings;
pub use crate::state::State;
pub use crate::state::TransitionCause;
pub use crate::time::TimeOfDay;
