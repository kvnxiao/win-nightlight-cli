mod consts;
pub mod nightlight_settings;
pub mod nightlight_state;
mod parser;

use nightlight_settings::NightlightSettings;
use nightlight_state::NightlightState;
use parser::DeserializationError;
use thiserror::Error;
use windows_registry::{CURRENT_USER, Type, Value};

const SETTINGS_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.settings\windows.data.bluelightreduction.settings";
const STATE_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\CloudStore\Store\DefaultAccount\Current\default$windows.data.bluelightreduction.bluelightreductionstate\windows.data.bluelightreduction.bluelightreductionstate";

#[derive(Error, Debug)]
pub enum NightlightError {
    #[error("Failed to open registry key")]
    OpenRegistryKey,
    #[error("Failed to read registry value")]
    ReadRegistryValue,
    #[error("Failed to write registry value")]
    WriteRegistryValue,
    #[error("Failed to deserialize data: {0}")]
    DeserializeData(DeserializationError),
}

fn get_raw_nightlight_bytes() -> Result<Vec<u8>, NightlightError> {
    let settings_key = CURRENT_USER
        .open(SETTINGS_REG_KEY)
        .map_err(|_| NightlightError::OpenRegistryKey)?;
    let data: Value = settings_key
        .get_value("Data")
        .map_err(|_| NightlightError::ReadRegistryValue)?;
    let data_vec: Vec<u8> = data.to_vec();
    Ok(data_vec)
}

fn set_raw_nightlight_bytes(bytes: &[u8]) -> Result<(), NightlightError> {
    let settings_key = CURRENT_USER
        .open(SETTINGS_REG_KEY)
        .map_err(|_| NightlightError::OpenRegistryKey)?;
    settings_key
        .set_bytes("Data", Type::Bytes, bytes)
        .map_err(|_| NightlightError::WriteRegistryValue)?;
    Ok(())
}

fn get_raw_nightlight_state_bytes() -> Result<Vec<u8>, NightlightError> {
    let state_key = CURRENT_USER
        .open(STATE_REG_KEY)
        .map_err(|_| NightlightError::OpenRegistryKey)?;
    let data: Value = state_key
        .get_value("Data")
        .map_err(|_| NightlightError::ReadRegistryValue)?;
    let data_vec: Vec<u8> = data.to_vec();
    Ok(data_vec)
}

fn set_raw_nightlight_state_bytes(bytes: &[u8]) -> Result<(), NightlightError> {
    let state_key = CURRENT_USER
        .open(STATE_REG_KEY)
        .map_err(|_| NightlightError::OpenRegistryKey)?;
    state_key
        .set_bytes("Data", Type::Bytes, bytes)
        .map_err(|_| NightlightError::WriteRegistryValue)?;
    Ok(())
}

pub fn get_nightlight_settings() -> Result<NightlightSettings, NightlightError> {
    let settings_bytes = get_raw_nightlight_bytes()?;
    NightlightSettings::deserialize_from_bytes(&settings_bytes)
        .map_err(NightlightError::DeserializeData)
}

pub fn set_nightlight_settings(settings: NightlightSettings) -> Result<(), NightlightError> {
    let settings_bytes = settings.serialize_to_bytes();
    set_raw_nightlight_bytes(&settings_bytes)?;
    Ok(())
}

pub fn get_nightlight_state() -> Result<NightlightState, NightlightError> {
    let state_bytes = get_raw_nightlight_state_bytes()?;
    NightlightState::deserialize_from_bytes(&state_bytes).map_err(NightlightError::DeserializeData)
}

pub fn set_nightlight_state(state: NightlightState) -> Result<(), NightlightError> {
    let state_bytes = state.serialize_to_bytes();
    set_raw_nightlight_state_bytes(&state_bytes)?;
    Ok(())
}
