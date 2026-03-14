pub(crate) mod bond;
mod cloudstore;
pub mod nightlight_settings;
pub mod nightlight_state;

pub use bond::BondError;

use chrono::NaiveTime;
use nightlight_settings::{NightlightSettings, ScheduleMode, SettingsError};
use nightlight_state::NightlightState;
use thiserror::Error;
use windows_registry::{CURRENT_USER, Value};
use windows_result::Error as WindowsError;

const SETTINGS_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings";
const STATE_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate";
const DATA_REG_KEY_NAME: &str = "Data";

#[derive(Error, Debug)]
pub enum NightlightError {
    #[error("Failed to open registry key")]
    OpenRegistryKey(WindowsError),
    #[error("Failed to read registry value")]
    ReadRegistryValue(WindowsError),
    #[error("Failed to write registry value")]
    WriteRegistryValue(WindowsError),
    #[error("Failed to deserialize data: {0}")]
    DeserializeData(BondError),
    #[error("{0}")]
    InvalidSettings(#[from] SettingsError),
}

/// Abstraction over the registry backend for reading/writing nightlight data.
pub trait NightlightBackend {
    fn read_settings_bytes(&self) -> Result<Vec<u8>, NightlightError>;
    fn write_settings_bytes(&self, data: &[u8]) -> Result<(), NightlightError>;
    fn read_state_bytes(&self) -> Result<Vec<u8>, NightlightError>;
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
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    // -- Primitive operations --

    pub fn get_settings(&self) -> Result<NightlightSettings, NightlightError> {
        let bytes = self.backend.read_settings_bytes()?;
        NightlightSettings::deserialize_from_bytes(&bytes).map_err(NightlightError::DeserializeData)
    }

    pub fn set_settings(&self, settings: &NightlightSettings) -> Result<(), NightlightError> {
        self.backend
            .write_settings_bytes(&settings.serialize_to_bytes())
    }

    pub fn get_state(&self) -> Result<NightlightState, NightlightError> {
        let bytes = self.backend.read_state_bytes()?;
        NightlightState::deserialize_from_bytes(&bytes).map_err(NightlightError::DeserializeData)
    }

    pub fn set_state(&self, state: &NightlightState) -> Result<(), NightlightError> {
        self.backend.write_state_bytes(&state.serialize_to_bytes())
    }

    // -- Composite operations --

    /// Enables nightlight (force-on), ignoring schedule mode.
    pub fn enable(&self) -> Result<(), NightlightError> {
        let mut state = self.get_state()?;
        if state.enable() {
            self.set_state(&state)?;
        }
        Ok(())
    }

    /// Disables nightlight and turns off any schedule.
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

    /// Sets the schedule mode, optionally overriding start/end times for manual mode.
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

        if changed {
            if mode != ScheduleMode::Off {
                let mut state = self.get_state()?;
                if state.enable() {
                    self.set_state(&state)?;
                }
            }
            self.set_settings(&settings)?;
        }
        Ok(())
    }

    /// Sets the color temperature (1200-6500 Kelvin).
    pub fn set_color_temperature(&self, temperature: u16) -> Result<(), NightlightError> {
        let mut settings = self.get_settings()?;
        if settings.set_color_temperature(temperature)? {
            self.set_settings(&settings)?;
        }
        Ok(())
    }
}

// -- Convenience free functions (backward compatibility) --

pub fn get_nightlight_settings() -> Result<NightlightSettings, NightlightError> {
    NightlightManager::new(RegistryBackend).get_settings()
}

pub fn set_nightlight_settings(settings: &NightlightSettings) -> Result<(), NightlightError> {
    NightlightManager::new(RegistryBackend).set_settings(settings)
}

pub fn get_nightlight_state() -> Result<NightlightState, NightlightError> {
    NightlightManager::new(RegistryBackend).get_state()
}

pub fn set_nightlight_state(state: &NightlightState) -> Result<(), NightlightError> {
    NightlightManager::new(RegistryBackend).set_state(state)
}
