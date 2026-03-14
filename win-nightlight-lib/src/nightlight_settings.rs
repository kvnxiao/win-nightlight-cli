use std::fmt;

use crate::bond::*;
use crate::cloudstore;
use chrono::{NaiveTime, Timelike, Utc};
use thiserror::Error;

/// Scheduling modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleMode {
    Off,
    SunsetToSunrise,
    SetHours,
}

impl fmt::Display for ScheduleMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleMode::Off => write!(f, "off"),
            ScheduleMode::SunsetToSunrise => write!(f, "sunset to sunrise"),
            ScheduleMode::SetHours => write!(f, "set hours"),
        }
    }
}

#[derive(Error, Debug)]
pub enum SettingsError {
    #[error("Invalid color temperature {0}")]
    InvalidColorTemperature(u16),
    #[error("Start/end times are only valid with manual schedule mode")]
    InvalidScheduleTimeOverride,
}

/// Night Light settings stored in the registry as a Bond CompactBinary v1 payload.
///
/// The binary format is a CloudStore wrapper containing an inner Bond struct with fields:
/// - Field 0:  bool   — schedule_enabled
/// - Field 10: bool   — set_hours_mode (presence = set hours mode)
/// - Field 20: struct — schedule start time (TimeBlock)
/// - Field 30: struct — schedule end time (TimeBlock)
/// - Field 40: int16  — color temperature (Kelvin)
/// - Field 50: struct — sunset time (TimeBlock)
/// - Field 60: struct — sunrise time (TimeBlock)
///
/// See `docs/nightlight-registry-format.md` for full details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NightlightSettings {
    /// The last-modified Unix timestamp in seconds
    pub timestamp: u64,
    /// The schedule mode
    pub schedule_mode: ScheduleMode,
    /// The color temperature in Kelvin
    pub color_temperature: u16,
    /// The start time of the schedule when [schedule_mode] is [ScheduleMode::SetHours]
    pub start_time: NaiveTime,
    /// The end time of the schedule when [schedule_mode] is [ScheduleMode::SetHours]
    pub end_time: NaiveTime,
    /// The sunset time
    pub sunset_time: NaiveTime,
    /// The sunrise time
    pub sunrise_time: NaiveTime,
}

/// Reads a TimeBlock struct: { field 0: int8 = hour, field 1: int8 = minute }.
/// Returns (hour, minute) with defaults of 0 for absent fields.
fn read_time_block(reader: &mut CompactBinaryReader) -> Result<(u8, u8), BondError> {
    let mut hour: u8 = 0;
    let mut minute: u8 = 0;
    loop {
        match reader.read_field_header()? {
            FieldHeader::Stop => break,
            FieldHeader::StopBase => continue,
            FieldHeader::Field { id: 0, bond_type: BondType::Int8 } => {
                hour = reader.read_int8()? as u8;
            }
            FieldHeader::Field { id: 1, bond_type: BondType::Int8 } => {
                minute = reader.read_int8()? as u8;
            }
            FieldHeader::Field { bond_type, .. } => {
                reader.skip_value(bond_type)?;
            }
        }
    }
    Ok((hour, minute))
}

/// Writes a TimeBlock struct. Omits fields with value 0 (Bond default omission).
fn write_time_block(writer: &mut CompactBinaryWriter, field_id: u16, hour: u8, minute: u8) {
    writer.write_field_header(field_id, BondType::Struct);
    if hour > 0 {
        writer.write_field_header(0, BondType::Int8);
        writer.write_int8(hour as i8);
    }
    if minute > 0 {
        writer.write_field_header(1, BondType::Int8);
        writer.write_int8(minute as i8);
    }
    writer.write_stop();
}

impl NightlightSettings {
    /// Deserializes a [NightlightSettings] struct from a byte slice.
    pub fn deserialize_from_bytes(data: &[u8]) -> Result<NightlightSettings, BondError> {
        let (timestamp, inner_payload) = cloudstore::cloudstore_unwrap(data)?;

        let mut reader = CompactBinaryReader::new(&inner_payload);
        reader.read_marshaled_header()?;

        let mut schedule_enabled = false;
        let mut set_hours_mode = false;
        let mut start_time = (0u8, 0u8);
        let mut end_time = (0u8, 0u8);
        let mut color_temperature: i16 = 0;
        let mut sunset_time = (0u8, 0u8);
        let mut sunrise_time = (0u8, 0u8);

        loop {
            match reader.read_field_header()? {
                FieldHeader::Stop => break,
                FieldHeader::StopBase => continue,
                FieldHeader::Field { id: 0, bond_type: BondType::Bool } => {
                    schedule_enabled = reader.read_bool()?;
                }
                FieldHeader::Field { id: 10, bond_type: BondType::Bool } => {
                    let _ = reader.read_bool()?;
                    set_hours_mode = true; // presence is the signal
                }
                FieldHeader::Field { id: 20, bond_type: BondType::Struct } => {
                    start_time = read_time_block(&mut reader)?;
                }
                FieldHeader::Field { id: 30, bond_type: BondType::Struct } => {
                    end_time = read_time_block(&mut reader)?;
                }
                FieldHeader::Field { id: 40, bond_type: BondType::Int16 } => {
                    color_temperature = reader.read_int16()?;
                }
                FieldHeader::Field { id: 50, bond_type: BondType::Struct } => {
                    sunset_time = read_time_block(&mut reader)?;
                }
                FieldHeader::Field { id: 60, bond_type: BondType::Struct } => {
                    sunrise_time = read_time_block(&mut reader)?;
                }
                FieldHeader::Field { bond_type, .. } => {
                    reader.skip_value(bond_type)?;
                }
            }
        }

        let schedule_mode = if schedule_enabled {
            if set_hours_mode {
                ScheduleMode::SetHours
            } else {
                ScheduleMode::SunsetToSunrise
            }
        } else {
            ScheduleMode::Off
        };

        let to_time = |h: u8, m: u8| -> Result<NaiveTime, BondError> {
            NaiveTime::from_hms_opt(u32::from(h), u32::from(m), 0)
                .ok_or(BondError::UnexpectedFieldType(0))
        };

        Ok(NightlightSettings {
            timestamp,
            schedule_mode,
            color_temperature: color_temperature as u16,
            start_time: to_time(start_time.0, start_time.1)?,
            end_time: to_time(end_time.0, end_time.1)?,
            sunset_time: to_time(sunset_time.0, sunset_time.1)?,
            sunrise_time: to_time(sunrise_time.0, sunrise_time.1)?,
        })
    }

    /// Serializes a [NightlightSettings] struct into a byte slice.
    pub fn serialize_to_bytes(&self) -> Vec<u8> {
        // Build inner payload
        let mut inner = CompactBinaryWriter::new();
        inner.write_marshaled_header();

        // Field 0: schedule_enabled
        if self.schedule_mode != ScheduleMode::Off {
            inner.write_field_header(0, BondType::Bool);
            inner.write_bool(true);
        }

        // Field 10: set_hours_mode (presence = set hours)
        if self.schedule_mode == ScheduleMode::SetHours {
            inner.write_field_header(10, BondType::Bool);
            inner.write_bool(false);
        }

        // Field 20: schedule start time
        write_time_block(
            &mut inner,
            20,
            self.start_time.hour() as u8,
            self.start_time.minute() as u8,
        );

        // Field 30: schedule end time
        write_time_block(
            &mut inner,
            30,
            self.end_time.hour() as u8,
            self.end_time.minute() as u8,
        );

        // Field 40: color temperature
        inner.write_field_header(40, BondType::Int16);
        inner.write_int16(self.color_temperature as i16);

        // Field 50: sunset time
        write_time_block(
            &mut inner,
            50,
            self.sunset_time.hour() as u8,
            self.sunset_time.minute() as u8,
        );

        // Field 60: sunrise time
        write_time_block(
            &mut inner,
            60,
            self.sunrise_time.hour() as u8,
            self.sunrise_time.minute() as u8,
        );

        inner.write_stop();

        cloudstore::cloudstore_wrap(self.timestamp, &inner.into_bytes())
    }

    fn update_timestamp(&mut self) {
        self.timestamp = Utc::now().timestamp() as u64;
    }

    /// Sets the schedule mode for the night light.
    pub fn set_mode(&mut self, mode: ScheduleMode) -> bool {
        if self.schedule_mode == mode {
            return false;
        }

        self.schedule_mode = mode;
        self.update_timestamp();
        true
    }

    /// Sets the color temperature for the night light, in a range between 1200 to 6500 Kelvin.
    pub fn set_color_temperature(
        &mut self,
        color_temperature: u16,
    ) -> Result<bool, SettingsError> {
        if self.color_temperature == color_temperature {
            return Ok(false);
        }

        if !(1200..=6500).contains(&color_temperature) {
            return Err(SettingsError::InvalidColorTemperature(color_temperature));
        }
        self.color_temperature = color_temperature;
        self.update_timestamp();
        Ok(true)
    }

    /// Sets the start time for the night light's set-hours schedule.
    pub fn set_start_time(&mut self, start_time: NaiveTime) -> bool {
        if self.start_time == start_time {
            return false;
        }

        self.start_time = start_time;
        self.update_timestamp();
        true
    }

    /// Sets the end time for the night light's set-hours schedule.
    pub fn set_end_time(&mut self, end_time: NaiveTime) -> bool {
        if self.end_time == end_time {
            return false;
        }

        self.end_time = end_time;
        self.update_timestamp();
        true
    }

    /// Sets the sunset time for the night light's sunset-to-sunrise schedule.
    pub fn set_sunset_time(&mut self, sunset_time: NaiveTime) -> bool {
        if self.sunset_time == sunset_time {
            return false;
        }

        self.sunset_time = sunset_time;
        self.update_timestamp();
        true
    }

    /// Sets the sunrise time for the night light's sunset-to-sunrise schedule.
    pub fn set_sunrise_time(&mut self, sunrise_time: NaiveTime) -> bool {
        if self.sunrise_time == sunrise_time {
            return false;
        }

        self.sunrise_time = sunrise_time;
        self.update_timestamp();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BYTES: [u8; 60] = [
        0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0xEC, 0xA0, 0xF4, 0xBE, 0x06,
        0x2A, 0x2B, 0x0E, 0x26, 0x43, 0x42, 0x01, 0x00, 0x02, 0x01, 0xC2, 0x0A, 0x00, 0xCA, 0x14,
        0x0E, 0x01, 0x2E, 0x0F, 0x00, 0xCA, 0x1E, 0x00, 0xCF, 0x28, 0xCC, 0x2B, 0xCA, 0x32, 0x0E,
        0x13, 0x2E, 0x17, 0x00, 0xCA, 0x3C, 0x0E, 0x07, 0x2E, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn test_serialize_to_bytes() {
        let settings = NightlightSettings {
            timestamp: 1742540908,
            schedule_mode: ScheduleMode::SetHours,
            color_temperature: 2790,
            start_time: NaiveTime::from_hms_opt(1, 15, 00).unwrap(),
            end_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(19, 23, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(7, 12, 0).unwrap(),
        };
        let bytes = settings.serialize_to_bytes();
        assert_eq!(BYTES, bytes.as_slice());
    }

    #[test]
    fn test_deserialize_from_bytes() {
        let expected_settings = NightlightSettings {
            timestamp: 1742540908,
            schedule_mode: ScheduleMode::SetHours,
            color_temperature: 2790,
            start_time: NaiveTime::from_hms_opt(1, 15, 00).unwrap(),
            end_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(19, 23, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(7, 12, 0).unwrap(),
        };
        let settings = NightlightSettings::deserialize_from_bytes(&BYTES).unwrap();
        assert_eq!(expected_settings, settings);
    }

    #[test]
    fn test_serde_roundtrip() {
        let settings = NightlightSettings {
            timestamp: 1742541024,
            schedule_mode: ScheduleMode::SetHours,
            color_temperature: 6500,
            start_time: NaiveTime::from_hms_opt(0, 15, 00).unwrap(),
            end_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(18, 26, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
        };
        let bytes = settings.serialize_to_bytes();
        let settings_from_bytes = NightlightSettings::deserialize_from_bytes(&bytes).unwrap();
        assert_eq!(settings, settings_from_bytes);
    }

    #[test]
    fn test_serde_roundtrip_schedule_off() {
        let settings = NightlightSettings {
            timestamp: 1742541024,
            schedule_mode: ScheduleMode::Off,
            color_temperature: 3400,
            start_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(6, 30, 0).unwrap(),
        };
        let bytes = settings.serialize_to_bytes();
        let roundtripped = NightlightSettings::deserialize_from_bytes(&bytes).unwrap();
        assert_eq!(settings, roundtripped);
    }

    #[test]
    fn test_serde_roundtrip_sunset_to_sunrise() {
        let settings = NightlightSettings {
            timestamp: 1742541024,
            schedule_mode: ScheduleMode::SunsetToSunrise,
            color_temperature: 1200,
            start_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(20, 45, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(5, 15, 0).unwrap(),
        };
        let bytes = settings.serialize_to_bytes();
        let roundtripped = NightlightSettings::deserialize_from_bytes(&bytes).unwrap();
        assert_eq!(settings, roundtripped);
    }
}
