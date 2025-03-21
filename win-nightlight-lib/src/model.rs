use chrono::{NaiveTime, Timelike};
use thiserror::Error;

/// Identifies the beginning of the struct
const STRUCT_HEADER_BYTES: [u8; 4] = [0x43, 0x42, 0x01, 0x00];
/// Identifies the end of the struct
const STRUCT_FOOTER_BYTES: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// Identifies the beginning of the timestamp struct
const TIMESTAMP_HEADER_BYTES: [u8; 4] = [0x0A, 0x02, 0x01, 0x00];
/// Identifies the start of the timestamp definition, and is always followed by the actual timestamp value
const TIMESTAMP_PREFIX_BYTES: [u8; 2] = [0x2A, 0x06];
/// The size of the timestamp struct in bytes
const TIMESTAMP_SIZE: usize = 5;
/// Identifies the end of the timestamp definition, and will always be preceded by the timestamp value
const TIMESTAMP_SUFFIX_BYTES: [u8; 3] = [0x2A, 0x2B, 0x0E];

/// These constant bytes will exist if scheduled mode is enabled in general (regardless if it's "sunset to sunrise" or "set hours")
const SCHEDULE_ENABLED_BYTES: [u8; 2] = [0x02, 0x01];
/// These constant bytes will exist specifically if "set hours" mode is enabled, and will always be preceded by [SCHEDULE_ENABLED_BYTES]
const SCHEDULE_MODE_SET_HOURS_ENABLED_BYTES: [u8; 3] = [0xC2, 0x0A, 0x00];

/// Identifies where the start time value definition begins
const SCHEDULE_START_TIME_PREFIX_BYTES: [u8; 2] = [0xCA, 0x14];
/// Identifies where the end time value definition begins
const SCHEDULE_END_TIME_PREFIX_BYTES: [u8; 2] = [0xCA, 0x1E];
/// Identifies where the sunset time value definition begins
const SUNSET_TIME_PREFIX_BYTES: [u8; 2] = [0xCA, 0x32];
/// Identifies where the sunrise time value definition begins
const SUNRISE_TIME_PREFIX_BYTES: [u8; 2] = [0xCA, 0x3C];
/// Identifies the next byte as the hour in a time block definition
const TIME_BLOCK_HOUR_IDENTIFIER_PREFIX_BYTE: u8 = 0x0E;
/// Identifies the next byte as the minute in a time block definition
const TIME_BLOCK_MINUTE_IDENTIFIER_PREFIX_BYTE: u8 = 0x2E;
/// Identifies the end of a time block definition
const TIME_BLOCK_TERMINAL_BYTE: u8 = 0x00;
/// Identifies where the color temperature value definition begins
const COLOR_TEMPERATURE_PREFIX_BYTES: [u8; 2] = [0xCF, 0x28];
/// The size of the color temperature definition in bytes
const COLOR_TEMPERATURE_SIZE: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleMode {
    Off,
    SunsetToSunrise,
    SetHours,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeBlockType {
    ScheduleStart,
    ScheduleEnd,
    Sunset,
    Sunrise,
}

impl TimeBlockType {
    fn get_prefix_identifier(&self) -> [u8; 2] {
        match self {
            TimeBlockType::ScheduleStart => SCHEDULE_START_TIME_PREFIX_BYTES,
            TimeBlockType::ScheduleEnd => SCHEDULE_END_TIME_PREFIX_BYTES,
            TimeBlockType::Sunset => SUNSET_TIME_PREFIX_BYTES,
            TimeBlockType::Sunrise => SUNRISE_TIME_PREFIX_BYTES,
        }
    }
}

/// The windows.data.bluelightreduction.settings data structure has the following binary format:
///
/// * [STRUCT_HEADER_BYTES]
/// * [TIMESTAMP_HEADER_BYTES]
/// * [TIMESTAMP_PREFIX_BYTES]
/// * The last-modified Unix timestamp in seconds, variably-encoded into [TIMESTAMP_SIZE] bytes
///     - byte 0: bits 0-6 = timestamp's bits 0-6, but top bit 7 is always set
///     - byte 1: bits 0-6 = timestamp's bits 7-13, but top bit 7 is always set
///     - byte 2: bits 0-6 = timestamp's bits 14-20, but top bit 7 is always set
///     - byte 3: bits 0-6 = timestamp's bits 21-27, but top bit 7 is always set
///     - byte 4: bits 0-6 = timestamp's bits 28-31, but top bit 7 is NOT set
/// * [TIMESTAMP_SUFFIX_BYTES]
/// * single byte to identify the length of the remaining data (schedule times and color temperature)
/// * [STRUCT_HEADER_BYTES] again
/// * if schedule == enabled: then include [SCHEDULE_MODE_ENABLED_BYTES]
/// * if schedule == enabled and is of type set_hours: then include [SCHEDULE_MODE_SET_HOURS_ENABLED_BYTES]
/// * [SCHEDULE_START_TIME_PREFIX_BYTES]
/// * variable encoding for start_time hour and minute (see below for more info.)
/// * [TIME_TERMINAL_BYTE]
/// * [SCHEDULE_END_TIME_PREFIX_BYTES]
/// * variable encoding for end_time hour and minute (see below for more info.)
/// * [TIME_TERMINAL_BYTE]
/// * [COLOR_TEMPERATURE_PREFIX_BYTES]
/// * color_temperature in Kelvin (1200-6500), variably encoded into [COLOR_TEMPERATURE_SIZE] bytes
///     - byte 0: bit 0 is always unset, bits 1-6 = temperature's bits 0-5, and bit 7 is always set
///     - byte 1: temperature's bit 6 and above, top bit not set
/// * [SUNSET_TIME_PREFIX_BYTES]
/// * variable encoding for sunset hour and minute (see below for more info.)
/// * [TIME_TERMINAL_BYTE]
/// * [SUNRISE_TIME_PREFIX_BYTES]
/// * variable encoding for sunrise hour and minute (see below for more info.)
/// * [TIME_TERMINAL_BYTE]
/// * [STRUCT_FOOTER_BYTES]
///
/// In terms of time blocks, the current known types are:
/// * [TimeBlockType::ScheduleStart]
/// * [TimeBlockType::ScheduleEnd]
/// * [TimeBlockType::Sunset]
/// * [TimeBlockType::Sunrise]
///
/// These time blocks are represented in the following variable-length binary format:
/// * 2 byte constant identifier based on known [TimeBlockType]s
/// * if hour > 0, then include:
///   - [TIME_BLOCK_HOUR_IDENTIFIER_PREFIX_BYTE] + hour value as a u8 (in the range of 0-23)
/// * if minute > 0, then include:
///   - [TIME_BLOCK_MINUTE_IDENTIFIER_PREFIX_BYTE] + minute value as a u8 (in the range of 0-59)
/// * [TIME_BLOCK_TERMINAL_BYTE]
///
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

#[derive(Error, Debug)]
pub enum DeserializationError {
    #[error("Invalid struct start")]
    StructStart,
    #[error("Invalid struct end")]
    StructEnd,
    #[error("Invalid timestamp block")]
    TimestampBlock,
    #[error("Invalid array conversion")]
    SliceArrayConversion,
    #[error("Invalid schedule start block")]
    ScheduleStartBlock,
    #[error("Invalid schedule end block")]
    ScheduleEndBlock,
    #[error("Invalid color temperature block")]
    ColorTemperatureBlock,
    #[error("Invalid sunset time block")]
    SunsetTimeBlock,
    #[error("Invalid sunrise time block")]
    SunriseTimeBlock,
    #[error("Invalid time value")]
    InvalidTimeValue,
    #[error("Invalid time block")]
    InvalidTimeBlock,
}

impl NightlightSettings {
    /// Converts a time block's hour and minute values to a [NaiveTime].
    fn time_to_naive_time(hours: u8, minutes: u8) -> Result<NaiveTime, DeserializationError> {
        NaiveTime::from_hms_opt(u32::from(hours), u32::from(minutes), 0)
            .ok_or(DeserializationError::InvalidTimeValue)
    }

    /// Parses the hour and minute values from the current time block position.
    fn time_hours_minutes_from_bytes(
        data: &[u8],
        pos: usize,
    ) -> Result<(u8, u8, usize), DeserializationError> {
        let mut pos = pos;

        // Check if the hour identifier byte exists
        let start_hour = if data[pos] == TIME_BLOCK_HOUR_IDENTIFIER_PREFIX_BYTE {
            let hour = data[pos + 1];
            pos += 2;
            hour
        } else {
            0
        };
        if start_hour >= 24 {
            return Err(DeserializationError::InvalidTimeBlock);
        }

        // Check if the minute identifier byte exists
        let start_minute = if data[pos] == TIME_BLOCK_MINUTE_IDENTIFIER_PREFIX_BYTE {
            let minute = data[pos + 1];
            pos += 2;
            minute
        } else {
            0
        };
        if start_minute >= 60 {
            return Err(DeserializationError::InvalidTimeBlock);
        }

        // Check if the end of time definition is reached
        if data[pos] != TIME_BLOCK_TERMINAL_BYTE {
            return Err(DeserializationError::InvalidTimeBlock);
        }

        // Check end of time definition
        if data[pos] != TIME_BLOCK_TERMINAL_BYTE {
            return Err(DeserializationError::InvalidTimeBlock);
        }
        pos += 1;

        Ok((start_hour, start_minute, pos))
    }

    /// Converts a [NaiveTime] to the expected binary byte slice representation.
    fn naive_time_to_bytes(time: NaiveTime, time_type: TimeBlockType) -> Vec<u8> {
        let mut bytes = Vec::new();
        let hour = time.hour() as u8;
        let minute = time.minute() as u8;

        bytes.extend_from_slice(&time_type.get_prefix_identifier());
        if hour > 0 {
            bytes.push(TIME_BLOCK_HOUR_IDENTIFIER_PREFIX_BYTE);
            bytes.push(hour);
        }
        if minute > 0 {
            bytes.push(TIME_BLOCK_MINUTE_IDENTIFIER_PREFIX_BYTE);
            bytes.push(minute);
        }
        bytes.push(TIME_BLOCK_TERMINAL_BYTE);
        bytes
    }

    /// Parses the struct header block.
    fn parse_struct_header_block(data: &[u8], pos: usize) -> Result<usize, DeserializationError> {
        if data[pos..pos + STRUCT_HEADER_BYTES.len()] != STRUCT_HEADER_BYTES {
            return Err(DeserializationError::StructStart);
        }
        Ok(pos + STRUCT_HEADER_BYTES.len())
    }

    /// Parses the last-modified timestamp block.
    fn parse_last_modified_timestamp_block(
        data: &[u8],
        start_from: usize,
    ) -> Result<(u64, usize), DeserializationError> {
        let mut pos: usize = start_from;
        // Check timestamp header bytes
        if data[pos..pos + TIMESTAMP_HEADER_BYTES.len()] != TIMESTAMP_HEADER_BYTES {
            return Err(DeserializationError::TimestampBlock);
        }
        pos += TIMESTAMP_HEADER_BYTES.len();
        // Check timestamp prefix bytes
        if data[pos..pos + TIMESTAMP_PREFIX_BYTES.len()] != TIMESTAMP_PREFIX_BYTES {
            return Err(DeserializationError::TimestampBlock);
        }
        pos += TIMESTAMP_PREFIX_BYTES.len();

        // Parse timestamp from bytes
        let timestamp_slice: [u8; TIMESTAMP_SIZE] = data[pos..pos + TIMESTAMP_SIZE]
            .try_into()
            .map_err(|_| DeserializationError::SliceArrayConversion)?;
        pos += TIMESTAMP_SIZE;
        let timestamp = Self::timestamp_from_bytes(timestamp_slice);

        // Check timestamp suffix bytes
        if data[pos..pos + TIMESTAMP_SUFFIX_BYTES.len()] != TIMESTAMP_SUFFIX_BYTES {
            return Err(DeserializationError::TimestampBlock);
        }
        pos += TIMESTAMP_SUFFIX_BYTES.len();

        Ok((timestamp, pos))
    }

    /// Checks if the schedule is enabled.
    fn parse_is_schedule_enabled_block(data: &[u8], pos: usize) -> (bool, usize) {
        match data[pos..pos + SCHEDULE_ENABLED_BYTES.len()] != SCHEDULE_ENABLED_BYTES {
            true => (false, pos),
            false => (true, pos + SCHEDULE_ENABLED_BYTES.len()),
        }
    }

    /// Checks if the schedule mode is set to "set hours".
    fn parse_is_schedule_mode_set_hours_enabled_block(data: &[u8], pos: usize) -> (bool, usize) {
        match data[pos..pos + SCHEDULE_MODE_SET_HOURS_ENABLED_BYTES.len()]
            != SCHEDULE_MODE_SET_HOURS_ENABLED_BYTES
        {
            true => (false, pos),
            false => (true, pos + SCHEDULE_MODE_SET_HOURS_ENABLED_BYTES.len()),
        }
    }

    /// Parses an arbitrary time block using the provided [TimeBlockType].
    fn parse_time_type_block(
        data: &[u8],
        pos: usize,
        time_type: TimeBlockType,
    ) -> Result<(u8, u8, usize), DeserializationError> {
        let prefix_bytes = time_type.get_prefix_identifier();
        if data[pos..pos + prefix_bytes.len()] != prefix_bytes {
            match time_type {
                TimeBlockType::ScheduleStart => {
                    return Err(DeserializationError::ScheduleStartBlock);
                }
                TimeBlockType::ScheduleEnd => return Err(DeserializationError::ScheduleEndBlock),
                TimeBlockType::Sunset => return Err(DeserializationError::SunsetTimeBlock),
                TimeBlockType::Sunrise => return Err(DeserializationError::SunriseTimeBlock),
            }
        }
        let (hours, minutes, pos) =
            Self::time_hours_minutes_from_bytes(data, pos + prefix_bytes.len())?;
        Ok((hours, minutes, pos))
    }

    /// Parses the color temperature block.
    fn parse_color_temperature_block(
        data: &[u8],
        pos: usize,
    ) -> Result<(u16, usize), DeserializationError> {
        let mut pos = pos;
        if data[pos..pos + COLOR_TEMPERATURE_PREFIX_BYTES.len()] != COLOR_TEMPERATURE_PREFIX_BYTES {
            return Err(DeserializationError::ColorTemperatureBlock);
        }
        pos += COLOR_TEMPERATURE_PREFIX_BYTES.len();
        let color_temperature_slice: [u8; COLOR_TEMPERATURE_SIZE] = data
            [pos..pos + COLOR_TEMPERATURE_SIZE]
            .try_into()
            .map_err(|_| DeserializationError::SliceArrayConversion)?;
        let color_temperature = Self::kelvin_from_bytes(color_temperature_slice);
        pos += COLOR_TEMPERATURE_SIZE;
        Ok((color_temperature, pos))
    }

    fn parse_struct_footer_block(data: &[u8], pos: usize) -> Result<usize, DeserializationError> {
        if data[pos..pos + STRUCT_FOOTER_BYTES.len()] != STRUCT_FOOTER_BYTES {
            return Err(DeserializationError::StructEnd);
        }
        Ok(pos + STRUCT_FOOTER_BYTES.len())
    }

    /// Deserializes a [NightlightSettings] struct from a byte slice.
    /// See [NightlightSettings] for more information about the format.
    pub fn deserialize_from_bytes(data: &[u8]) -> Result<NightlightSettings, DeserializationError> {
        let pos = Self::parse_struct_header_block(data, 0)?;
        let (timestamp, pos) = Self::parse_last_modified_timestamp_block(data, pos)?;

        // Check if the remaining struct size is valid
        let remaining_struct_size: u8 = data[pos];
        if data.len() != remaining_struct_size as usize + pos + STRUCT_FOOTER_BYTES.len() {
            return Err(DeserializationError::StructEnd);
        }

        let pos = Self::parse_struct_header_block(data, pos + 1)?; // skip 1 byte since we read remaining_struct_size
        let (is_schedule_enabled, pos) = Self::parse_is_schedule_enabled_block(data, pos);
        let (is_schedule_mode_set_hours_enabled, pos) =
            Self::parse_is_schedule_mode_set_hours_enabled_block(data, pos);
        let (start_hour, start_minute, pos) =
            Self::parse_time_type_block(data, pos, TimeBlockType::ScheduleStart)?;
        let (end_hour, end_minute, pos) =
            Self::parse_time_type_block(data, pos, TimeBlockType::ScheduleEnd)?;
        let (color_temperature, pos) = Self::parse_color_temperature_block(data, pos)?;
        let (sunset_hour, sunset_minute, pos) =
            Self::parse_time_type_block(data, pos, TimeBlockType::Sunset)?;
        let (sunrise_hour, sunrise_minute, pos) =
            Self::parse_time_type_block(data, pos, TimeBlockType::Sunrise)?;
        let pos = Self::parse_struct_footer_block(data, pos)?;

        if pos != data.len() {
            return Err(DeserializationError::StructEnd);
        }

        let schedule_mode = if is_schedule_enabled {
            if is_schedule_mode_set_hours_enabled {
                ScheduleMode::SetHours
            } else {
                ScheduleMode::SunsetToSunrise
            }
        } else {
            ScheduleMode::Off
        };

        let start_time = Self::time_to_naive_time(start_hour, start_minute)?;
        let end_time = Self::time_to_naive_time(end_hour, end_minute)?;
        let sunset_time = Self::time_to_naive_time(sunset_hour, sunset_minute)?;
        let sunrise_time = Self::time_to_naive_time(sunrise_hour, sunrise_minute)?;

        let settings = NightlightSettings {
            timestamp,
            schedule_mode,
            color_temperature,
            start_time,
            end_time,
            sunset_time,
            sunrise_time,
        };
        Ok(settings)
    }

    /// Converts a Unix timestamp to a 5-byte array using a variable-length encoding scheme.
    /// See [NightlightSettings] for more information about the format.
    pub fn timestamp_to_bytes(&self) -> [u8; TIMESTAMP_SIZE] {
        let mut bytes: [u8; TIMESTAMP_SIZE] = [0; TIMESTAMP_SIZE];
        bytes[0] = (self.timestamp & 0x7F | 0x80) as u8;
        bytes[1] = ((self.timestamp >> 7) & 0x7F | 0x80) as u8;
        bytes[2] = ((self.timestamp >> 14) & 0x7F | 0x80) as u8;
        bytes[3] = ((self.timestamp >> 21) & 0x7F | 0x80) as u8;
        bytes[4] = (self.timestamp >> 28) as u8;
        bytes
    }

    /// Converts a 5-byte array to a Unix timestamp using a variable-length decoding scheme.
    /// See [NightlightSettings] for more information about the format.
    pub fn timestamp_from_bytes(bytes: [u8; TIMESTAMP_SIZE]) -> u64 {
        let mut timestamp: u64 = 0;
        timestamp |= (bytes[4] as u64) << 28;
        timestamp |= ((bytes[3] & 0x7F) as u64) << 21;
        timestamp |= ((bytes[2] & 0x7F) as u64) << 14;
        timestamp |= ((bytes[1] & 0x7F) as u64) << 7;
        timestamp |= (bytes[0] & 0x7F) as u64;
        timestamp
    }

    /// Converts a color temperature in Kelvin to a 2-byte array using a mangled encoding scheme.
    /// See [NightlightSettings] for more information about the format.
    pub fn kelvin_to_bytes(&self) -> [u8; 2] {
        let mut bytes: [u8; 2] = [0; 2];
        bytes[0] = ((self.color_temperature & 0x3F) * 2 + 0x80) as u8;
        bytes[1] = (self.color_temperature >> 6) as u8;
        bytes
    }

    /// Converts a 2-byte array to a color temperature in Kelvin using a mangled decoding scheme.
    /// See [NightlightSettings] for more information about the format.
    pub fn kelvin_from_bytes(bytes: [u8; 2]) -> u16 {
        let mut kelvin: u16 = 0;
        kelvin |= (bytes[1] as u16) << 6;
        kelvin |= ((bytes[0] - 0x80) / 2) as u16;
        kelvin
    }

    pub fn serialize_to_bytes(&self) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(&STRUCT_HEADER_BYTES);
        bytes.extend_from_slice(&TIMESTAMP_HEADER_BYTES);
        bytes.extend_from_slice(&TIMESTAMP_PREFIX_BYTES);
        bytes.extend_from_slice(&self.timestamp_to_bytes());
        bytes.extend_from_slice(&TIMESTAMP_SUFFIX_BYTES);

        // Figure out the size of the remaining bytes after computing the back bytes
        let mut remaining_struct_bytes: Vec<u8> = Vec::new();
        remaining_struct_bytes.extend_from_slice(&STRUCT_HEADER_BYTES);
        match self.schedule_mode {
            ScheduleMode::Off => {
                // no-op
            }
            ScheduleMode::SunsetToSunrise => {
                remaining_struct_bytes.extend_from_slice(&SCHEDULE_ENABLED_BYTES);
            }
            ScheduleMode::SetHours => {
                remaining_struct_bytes.extend_from_slice(&SCHEDULE_ENABLED_BYTES);
                remaining_struct_bytes.extend_from_slice(&SCHEDULE_MODE_SET_HOURS_ENABLED_BYTES);
            }
        }

        let start_time_bytes =
            Self::naive_time_to_bytes(self.start_time, TimeBlockType::ScheduleStart);
        let end_time_bytes = Self::naive_time_to_bytes(self.end_time, TimeBlockType::ScheduleEnd);
        let color_temperature_bytes = self.kelvin_to_bytes();
        let sunset_time_bytes = Self::naive_time_to_bytes(self.sunset_time, TimeBlockType::Sunset);
        let sunrise_time_bytes =
            Self::naive_time_to_bytes(self.sunrise_time, TimeBlockType::Sunrise);

        remaining_struct_bytes.extend_from_slice(&start_time_bytes);
        remaining_struct_bytes.extend_from_slice(&end_time_bytes);
        remaining_struct_bytes.extend_from_slice(&COLOR_TEMPERATURE_PREFIX_BYTES);
        remaining_struct_bytes.extend_from_slice(&color_temperature_bytes);
        remaining_struct_bytes.extend_from_slice(&sunset_time_bytes);
        remaining_struct_bytes.extend_from_slice(&sunrise_time_bytes);

        let remaining_struct_size = remaining_struct_bytes.len() as u8 + 1;
        bytes.push(remaining_struct_size);
        bytes.extend(remaining_struct_bytes);
        bytes.extend_from_slice(&STRUCT_FOOTER_BYTES);

        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_roundtrip_conversion() {
        let settings = NightlightSettings {
            timestamp: 1742518000,
            schedule_mode: ScheduleMode::Off,
            color_temperature: 2700,
            start_time: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        };
        let bytes = settings.timestamp_to_bytes();
        let timestamp_from_bytes = NightlightSettings::timestamp_from_bytes(bytes);
        assert_eq!(settings.timestamp, timestamp_from_bytes);
    }

    #[test]
    fn test_kelvin_roundtrip_conversion() {
        let settings = NightlightSettings {
            timestamp: 1742518000,
            schedule_mode: ScheduleMode::Off,
            color_temperature: 1200,
            start_time: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            end_time: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        };
        let bytes = settings.kelvin_to_bytes();
        let kelvin_from_bytes = NightlightSettings::kelvin_from_bytes(bytes);
        assert_eq!(settings.color_temperature, kelvin_from_bytes);
    }

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
        let expected_bytes: [u8; 60] = [
            0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0xEC, 0xA0, 0xF4, 0xBE,
            0x06, 0x2A, 0x2B, 0x0E, 0x26, 0x43, 0x42, 0x01, 0x00, 0x02, 0x01, 0xC2, 0x0A, 0x00,
            0xCA, 0x14, 0x0E, 0x01, 0x2E, 0x0F, 0x00, 0xCA, 0x1E, 0x00, 0xCF, 0x28, 0xCC, 0x2B,
            0xCA, 0x32, 0x0E, 0x13, 0x2E, 0x17, 0x00, 0xCA, 0x3C, 0x0E, 0x07, 0x2E, 0x0C, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let bytes = settings.serialize_to_bytes();
        let bytes_slice: &[u8] = bytes.as_slice();
        assert_eq!(expected_bytes, bytes_slice);
    }

    #[test]
    fn test_deserialize_from_bytes() {
        let bytes: [u8; 60] = [
            0x43, 0x42, 0x01, 0x00, 0x0A, 0x02, 0x01, 0x00, 0x2A, 0x06, 0xEC, 0xA0, 0xF4, 0xBE,
            0x06, 0x2A, 0x2B, 0x0E, 0x26, 0x43, 0x42, 0x01, 0x00, 0x02, 0x01, 0xC2, 0x0A, 0x00,
            0xCA, 0x14, 0x0E, 0x01, 0x2E, 0x0F, 0x00, 0xCA, 0x1E, 0x00, 0xCF, 0x28, 0xCC, 0x2B,
            0xCA, 0x32, 0x0E, 0x13, 0x2E, 0x17, 0x00, 0xCA, 0x3C, 0x0E, 0x07, 0x2E, 0x0C, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let expected_settings = NightlightSettings {
            timestamp: 1742540908,
            schedule_mode: ScheduleMode::SetHours,
            color_temperature: 2790,
            start_time: NaiveTime::from_hms_opt(1, 15, 00).unwrap(),
            end_time: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            sunset_time: NaiveTime::from_hms_opt(19, 23, 0).unwrap(),
            sunrise_time: NaiveTime::from_hms_opt(7, 12, 0).unwrap(),
        };
        let settings = NightlightSettings::deserialize_from_bytes(&bytes).unwrap();
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
}
