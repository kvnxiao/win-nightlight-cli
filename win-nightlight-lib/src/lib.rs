//! Read and change Windows 11 Night Light in the current user's registry.
//!
//! Windows stores Night Light in two registry values:
//!
//! - [`Settings`]: the schedule and the color temperature.
//! - [`State`]: whether Night Light is on now, and when and why it last turned
//!   on or off.
//!
//! [`Nightlight`] writes settings and state separately. Temperature updates
//! can read both values and write only settings; on/off updates write only
//! state.
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
//! Show a color temperature now, or store it while Night Light is off, and
//! set a set-hours schedule:
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
//! let nightlight = Nightlight::new();
//! nightlight.apply_color_temperature(temperature)?;
//! nightlight.update_settings(|settings| {
//!     settings.set_schedule(Schedule::SetHours {
//!         start: Some(start),
//!         end: Some(end),
//!     });
//! })?;
//! # Ok::<(), win_nightlight_lib::Error>(())
//! ```
//!
//! Fade the temperature with a held preview, as dragging the Night Light
//! slider in Windows Settings does: turn preview on, write each step while
//! preview stays on, and turn preview off. The [color temperature transitions
//! guide] describes:
//!
//! - Which methods to use for one-off changes, ramps, and fades.
//! - How often to write.
//! - How far each write moves the settings timestamp ahead of real time.
//!
//! ```no_run
//! use std::time::Duration;
//! use win_nightlight_lib::ColorTemperature;
//! use win_nightlight_lib::Nightlight;
//!
//! let nightlight = Nightlight::new();
//! nightlight.start_preview()?;
//! let stepped = [5800, 5100, 4400, 3700].into_iter().try_for_each(|kelvin| {
//!     nightlight.preview_color_temperature(ColorTemperature::new(kelvin)?)?;
//!     std::thread::sleep(Duration::from_millis(250));
//!     Ok::<(), win_nightlight_lib::Error>(())
//! });
//! // End the preview even after a failed step: while preview is on, Windows
//! // shows the stored temperature even with Night Light off.
//! let ended = nightlight.end_preview();
//! stepped?;
//! ended?;
//! # Ok::<(), win_nightlight_lib::Error>(())
//! ```
//!
//! [color temperature transitions guide]: https://github.com/kvnxiao/win-nightlight-cli/blob/main/docs/color-temperature-transitions.md
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
pub use crate::store::SETTINGS_KEY_PATH;
pub use crate::store::STATE_KEY_PATH;
pub use crate::time::TimeOfDay;
