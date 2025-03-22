mod consts;
pub mod nightlight_settings;
pub mod nightlight_state;
mod parser;

use nightlight_settings::NightlightSettings;
use nightlight_state::NightlightState;
use parser::DeserializationError;
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
    #[error("Failed to convert bytes to registry value")]
    ConvertBytesToValue,
    #[error("Failed to deserialize data: {0}")]
    DeserializeData(DeserializationError),
}

fn get_raw_nightlight_bytes() -> Result<Vec<u8>, NightlightError> {
    let settings_key = CURRENT_USER
        .options()
        .read()
        .open(SETTINGS_REG_KEY)
        .map_err(NightlightError::OpenRegistryKey)?;
    let data: Value = settings_key
        .get_value(DATA_REG_KEY_NAME)
        .map_err(NightlightError::ReadRegistryValue)?;
    let data_vec: Vec<u8> = data.to_vec();
    Ok(data_vec)
}

fn set_raw_nightlight_bytes(bytes: &[u8]) -> Result<(), NightlightError> {
    let settings_key = CURRENT_USER
        .options()
        .write()
        .open(SETTINGS_REG_KEY)
        .map_err(NightlightError::OpenRegistryKey)?;
    let value = Value::from(bytes);
    settings_key
        .set_value(DATA_REG_KEY_NAME, &value)
        .map_err(NightlightError::WriteRegistryValue)?;
    Ok(())
}

pub fn get_raw_nightlight_state_bytes() -> Result<Vec<u8>, NightlightError> {
    let state_key = CURRENT_USER
        .options()
        .read()
        .open(STATE_REG_KEY)
        .map_err(NightlightError::OpenRegistryKey)?;
    let data: Value = state_key
        .get_value(DATA_REG_KEY_NAME)
        .map_err(NightlightError::ReadRegistryValue)?;
    let data_vec: Vec<u8> = data.to_vec();
    Ok(data_vec)
}

pub fn set_raw_nightlight_state_bytes(bytes: &[u8]) -> Result<(), NightlightError> {
    let state_key = CURRENT_USER
        .options()
        .write()
        .open(STATE_REG_KEY)
        .map_err(NightlightError::OpenRegistryKey)?;
    let value = Value::from(bytes);
    state_key
        .set_value(DATA_REG_KEY_NAME, &value)
        .map_err(NightlightError::WriteRegistryValue)?;
    Ok(())
}

pub fn get_nightlight_settings() -> Result<NightlightSettings, NightlightError> {
    let settings_bytes = get_raw_nightlight_bytes()?;
    NightlightSettings::deserialize_from_bytes(&settings_bytes)
        .map_err(NightlightError::DeserializeData)
}

pub fn set_nightlight_settings(settings: &NightlightSettings) -> Result<(), NightlightError> {
    let settings_bytes = settings.serialize_to_bytes();
    set_raw_nightlight_bytes(&settings_bytes)?;
    Ok(())
}

pub fn get_nightlight_state() -> Result<NightlightState, NightlightError> {
    let state_bytes = get_raw_nightlight_state_bytes()?;
    NightlightState::deserialize_from_bytes(&state_bytes).map_err(NightlightError::DeserializeData)
}

pub fn set_nightlight_state(state: &NightlightState) -> Result<(), NightlightError> {
    let state_bytes = state.serialize_to_bytes();
    set_raw_nightlight_state_bytes(&state_bytes)?;
    Ok(())
}
