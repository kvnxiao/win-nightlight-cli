//! Read and modify the Windows 11 Night Light state and settings stored in the
//! current user's registry.
//!
//! [`NightlightManager`] reads and writes both registry values through a
//! [`NightlightBackend`]; [`RegistryBackend`] targets the live registry.

pub(crate) mod bond;
mod cloudstore;
pub mod nightlight_settings;
pub mod nightlight_state;

pub use bond::BondError;
use chrono::NaiveTime;
use nightlight_settings::NightlightSettings;
use nightlight_settings::ScheduleMode;
use nightlight_settings::SettingsError;
use nightlight_state::NightlightState;
use thiserror::Error;
use windows_registry::CURRENT_USER;
use windows_registry::Value;
use windows_result::Error as WindowsError;

const SETTINGS_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings";
const STATE_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate";
const DATA_REG_KEY_NAME: &str = "Data";

/// Failure to read, decode, encode, or write Night Light registry data, or an
/// invalid setting value.
#[derive(Error, Debug)]
pub enum NightlightError {
    /// The Night Light registry key could not be opened.
    #[error("Failed to open registry key")]
    OpenRegistryKey(WindowsError),
    /// The registry value could not be read.
    #[error("Failed to read registry value")]
    ReadRegistryValue(WindowsError),
    /// The registry value could not be written.
    #[error("Failed to write registry value")]
    WriteRegistryValue(WindowsError),
    /// The registry value is not a valid Night Light payload.
    #[error("Failed to deserialize data: {0}")]
    DeserializeData(BondError),
    /// The Night Light data could not be encoded.
    #[error("Failed to serialize data")]
    SerializeData(#[source] BondError),
    /// A requested setting value is invalid.
    #[error("{0}")]
    InvalidSettings(#[from] SettingsError),
}

/// Abstraction over the registry backend for reading/writing nightlight data.
pub trait NightlightBackend {
    /// Reads the raw Night Light settings payload.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if the backend cannot read the payload.
    fn read_settings_bytes(&self) -> Result<Vec<u8>, NightlightError>;

    /// Writes the raw Night Light settings payload.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if the backend cannot write the payload.
    fn write_settings_bytes(&self, data: &[u8]) -> Result<(), NightlightError>;

    /// Reads the raw Night Light state payload.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if the backend cannot read the payload.
    fn read_state_bytes(&self) -> Result<Vec<u8>, NightlightError>;

    /// Writes the raw Night Light state payload.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if the backend cannot write the payload.
    fn write_state_bytes(&self, data: &[u8]) -> Result<(), NightlightError>;
}

/// Windows Registry backend implementation.
pub struct RegistryBackend;

impl RegistryBackend {
    fn read_registry_data(reg_key: &str) -> Result<Vec<u8>, NightlightError> {
        let key = CURRENT_USER
            .options()
            .read()
            .open(reg_key)
            .map_err(NightlightError::OpenRegistryKey)?;
        let data: Value = key
            .get_value(DATA_REG_KEY_NAME)
            .map_err(NightlightError::ReadRegistryValue)?;
        Ok(data.to_vec())
    }

    fn write_registry_data(reg_key: &str, bytes: &[u8]) -> Result<(), NightlightError> {
        let key = CURRENT_USER
            .options()
            .write()
            .open(reg_key)
            .map_err(NightlightError::OpenRegistryKey)?;
        key.set_value(DATA_REG_KEY_NAME, &Value::from(bytes))
            .map_err(NightlightError::WriteRegistryValue)
    }
}

impl NightlightBackend for RegistryBackend {
    fn read_settings_bytes(&self) -> Result<Vec<u8>, NightlightError> {
        Self::read_registry_data(SETTINGS_REG_KEY)
    }

    fn write_settings_bytes(&self, data: &[u8]) -> Result<(), NightlightError> {
        Self::write_registry_data(SETTINGS_REG_KEY, data)
    }

    fn read_state_bytes(&self) -> Result<Vec<u8>, NightlightError> {
        Self::read_registry_data(STATE_REG_KEY)
    }

    fn write_state_bytes(&self, data: &[u8]) -> Result<(), NightlightError> {
        Self::write_registry_data(STATE_REG_KEY, data)
    }
}

/// High-level interface for reading/writing Night Light settings and state.
pub struct NightlightManager<B: NightlightBackend> {
    backend: B,
}

impl<B: NightlightBackend> NightlightManager<B> {
    /// Creates a manager that reads and writes through `backend`.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    // -- Primitive operations --

    /// Reads the current Night Light settings.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if the backend read fails or the payload
    /// does not decode.
    pub fn get_settings(&self) -> Result<NightlightSettings, NightlightError> {
        let bytes = self.backend.read_settings_bytes()?;
        NightlightSettings::deserialize_from_bytes(&bytes).map_err(NightlightError::DeserializeData)
    }

    /// Writes `settings` as the current Night Light settings.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if `settings` does not encode or the
    /// backend write fails.
    pub fn set_settings(&self, settings: &NightlightSettings) -> Result<(), NightlightError> {
        let bytes = settings
            .serialize_to_bytes()
            .map_err(NightlightError::SerializeData)?;
        self.backend.write_settings_bytes(&bytes)
    }

    /// Reads the current Night Light state.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if the backend read fails or the payload
    /// does not decode.
    pub fn get_state(&self) -> Result<NightlightState, NightlightError> {
        let bytes = self.backend.read_state_bytes()?;
        NightlightState::deserialize_from_bytes(&bytes).map_err(NightlightError::DeserializeData)
    }

    /// Writes `state` as the current Night Light state.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if `state` does not encode or the backend
    /// write fails.
    pub fn set_state(&self, state: &NightlightState) -> Result<(), NightlightError> {
        let bytes = state
            .serialize_to_bytes()
            .map_err(NightlightError::SerializeData)?;
        self.backend.write_state_bytes(&bytes)
    }

    // -- Composite operations --

    /// Enables nightlight (force-on), ignoring schedule mode.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if reading or writing the state fails.
    pub fn enable(&self) -> Result<(), NightlightError> {
        let mut state = self.get_state()?;
        if state.enable() {
            self.set_state(&state)?;
        }
        Ok(())
    }

    /// Disables nightlight and turns off any schedule.
    ///
    /// # Errors
    ///
    /// Returns a [`NightlightError`] if reading or writing the settings or
    /// state fails.
    pub fn disable(&self) -> Result<(), NightlightError> {
        let mut settings = self.get_settings()?;
        if settings.set_mode(ScheduleMode::Off) {
            self.set_settings(&settings)?;
        }
        let mut state = self.get_state()?;
        if state.disable() {
            self.set_state(&state)?;
        }
        Ok(())
    }

    /// Sets the schedule mode, optionally overriding start/end times for manual
    /// mode.
    ///
    /// # Errors
    ///
    /// Returns [`NightlightError::InvalidSettings`] with
    /// [`SettingsError::InvalidScheduleTimeOverride`] if `start` or `end` is
    /// set for a mode other than [`ScheduleMode::SetHours`]. Returns another
    /// [`NightlightError`] if reading or writing the settings or state fails.
    pub fn set_schedule(
        &self,
        mode: ScheduleMode,
        start: Option<NaiveTime>,
        end: Option<NaiveTime>,
    ) -> Result<(), NightlightError> {
        if mode != ScheduleMode::SetHours && (start.is_some() || end.is_some()) {
            return Err(SettingsError::InvalidScheduleTimeOverride.into());
        }

        let mut settings = self.get_settings()?;
        let mut changed = settings.set_mode(mode);

        if let Some(t) = start {
            changed |= settings.set_start_time(t);
        }
        if let Some(t) = end {
            changed |= settings.set_end_time(t);
        }

        if !changed {
            return Ok(());
        }
        if mode != ScheduleMode::Off {
            let mut state = self.get_state()?;
            if state.enable() {
                self.set_state(&state)?;
            }
        }
        self.set_settings(&settings)
    }

    /// Sets the color temperature (1200-6500 Kelvin).
    ///
    /// # Errors
    ///
    /// Returns [`NightlightError::InvalidSettings`] with
    /// [`SettingsError::InvalidColorTemperature`] if `temperature` is out of
    /// range. Returns another [`NightlightError`] if reading or writing the
    /// settings fails.
    pub fn set_color_temperature(&self, temperature: u16) -> Result<(), NightlightError> {
        let mut settings = self.get_settings()?;
        if settings.set_color_temperature(temperature)? {
            self.set_settings(&settings)?;
        }
        Ok(())
    }
}

// -- Convenience free functions (backward compatibility) --

/// Reads the current Night Light settings from the registry.
///
/// # Errors
///
/// Returns a [`NightlightError`] if the registry read fails or the payload does
/// not decode.
pub fn get_nightlight_settings() -> Result<NightlightSettings, NightlightError> {
    NightlightManager::new(RegistryBackend).get_settings()
}

/// Writes `settings` to the registry.
///
/// # Errors
///
/// Returns a [`NightlightError`] if `settings` does not encode or the registry
/// write fails.
pub fn set_nightlight_settings(settings: &NightlightSettings) -> Result<(), NightlightError> {
    NightlightManager::new(RegistryBackend).set_settings(settings)
}

/// Reads the current Night Light state from the registry.
///
/// # Errors
///
/// Returns a [`NightlightError`] if the registry read fails or the payload does
/// not decode.
pub fn get_nightlight_state() -> Result<NightlightState, NightlightError> {
    NightlightManager::new(RegistryBackend).get_state()
}

/// Writes `state` to the registry.
///
/// # Errors
///
/// Returns a [`NightlightError`] if `state` does not encode or the registry
/// write fails.
pub fn set_nightlight_state(state: &NightlightState) -> Result<(), NightlightError> {
    NightlightManager::new(RegistryBackend).set_state(state)
}
