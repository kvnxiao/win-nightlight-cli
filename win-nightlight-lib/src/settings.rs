use crate::Error;
use crate::Result;
use crate::TimeOfDay;
use crate::bond::BondError;
use crate::bond::BondType;
use crate::bond::CompactBinaryReader;
use crate::bond::CompactBinaryWriter;
use crate::bond::StructWriter;
use crate::bond::UnknownFields;
use crate::bond::expect_type;
use crate::cloudstore::Envelope;
use crate::error::Repr;
use crate::store::Key;
use crate::time::system_time_from_unix_secs;
use std::fmt;
use std::str::FromStr;
use std::time::SystemTime;

const SCHEDULE_ENABLED: u16 = 0;
const ON_SUN_SCHEDULE: u16 = 10;
const SCHEDULE_START: u16 = 20;
const SCHEDULE_END: u16 = 30;
const COLOR_TEMPERATURE: u16 = 40;
const SUNSET: u16 = 50;
const SUNRISE: u16 = 60;
const PREVIEWING: u16 = 70;
const TIME_HOUR: u16 = 0;
const TIME_MINUTE: u16 = 1;

/// Night Light color temperature in kelvin, from 1200K to 6500K.
///
/// Parses from a whole number of kelvin with an optional `K` suffix and
/// displays with the suffix, as in `3400K`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColorTemperature(u16);

impl ColorTemperature {
    /// Warmest color temperature Windows accepts.
    pub const MIN: Self = Self(1200);
    /// Coolest color temperature Windows accepts, which applies no filter.
    pub const MAX: Self = Self(6500);

    /// Create a color temperature of `kelvin`.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`ErrorKind::InvalidInput`] if `kelvin` is
    /// outside [`MIN`](Self::MIN) to [`MAX`](Self::MAX).
    ///
    /// [`ErrorKind::InvalidInput`]: crate::ErrorKind::InvalidInput
    pub fn new(kelvin: u16) -> Result<Self> {
        if (Self::MIN.0..=Self::MAX.0).contains(&kelvin) {
            Ok(Self(kelvin))
        } else {
            Err(Repr::ColorTemperatureRange(u32::from(kelvin)).into())
        }
    }

    /// Return the color temperature in kelvin.
    pub const fn kelvin(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for ColorTemperature {
    type Error = Error;

    fn try_from(kelvin: u16) -> Result<Self> {
        Self::new(kelvin)
    }
}

impl FromStr for ColorTemperature {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let digits = text.strip_suffix(['K', 'k']).unwrap_or(text);
        let kelvin = digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse::<u32>().ok())
            .flatten()
            .ok_or_else(|| Repr::ColorTemperatureSyntax(text.to_owned()))?;
        u16::try_from(kelvin)
            .ok()
            .and_then(|kelvin| Self::new(kelvin).ok())
            .ok_or_else(|| Repr::ColorTemperatureRange(kelvin).into())
    }
}

impl fmt::Display for ColorTemperature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}K", self.0)
    }
}

/// Type of schedule, which Windows keeps while the schedule is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScheduleKind {
    /// Turn on at sunset and off at sunrise for the current location.
    SunsetToSunrise,
    /// Turn on and off at set times.
    SetHours,
}

/// Effective schedule, combining whether the schedule is on with its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScheduleMode {
    /// Night Light changes only when turned on or off by hand.
    Off,
    /// Night Light turns on at sunset and off at sunrise.
    SunsetToSunrise,
    /// Night Light turns on and off at set times.
    SetHours,
}

impl fmt::Display for ScheduleMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::SunsetToSunrise => "sunset to sunrise",
            Self::SetHours => "set hours",
        })
    }
}

/// Schedule change for [`Settings::set_schedule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Schedule {
    /// Turn the schedule off and keep its kind and times.
    Off,
    /// Turn the schedule on from sunset to sunrise.
    SunsetToSunrise,
    /// Turn the schedule on at set times, replacing each time that is
    /// `Some` and keeping the stored time for each that is `None`.
    SetHours {
        /// Time Night Light turns on.
        start: Option<TimeOfDay>,
        /// Time Night Light turns off.
        end: Option<TimeOfDay>,
    },
}

/// Night Light schedule and color temperature settings.
///
/// Decoding keeps unmodeled fields of the settings struct and its envelope,
/// and encoding writes them back, so a round trip changes only what a setter
/// changed. Unmodeled fields inside a schedule, sunset, or sunrise time are
/// not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    envelope: Envelope,
    schedule_enabled: bool,
    schedule_kind: ScheduleKind,
    schedule_start: TimeOfDay,
    schedule_end: TimeOfDay,
    color_temperature: Option<i16>,
    sunset: TimeOfDay,
    sunrise: TimeOfDay,
    previewing: bool,
    unknown: UnknownFields,
}

impl Settings {
    /// Decode settings from the registry value bytes.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`ErrorKind::Decode`] if `data` is not a valid
    /// Night Light settings payload.
    ///
    /// [`ErrorKind::Decode`]: crate::ErrorKind::Decode
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::decode(data).map_err(|source| {
            Repr::Decode {
                key: Key::Settings,
                source,
            }
            .into()
        })
    }

    /// Encode these settings as registry value bytes, keeping the stored
    /// last-modified time.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`ErrorKind::Encode`] if the payload is too
    /// large for the format.
    ///
    /// [`ErrorKind::Encode`]: crate::ErrorKind::Encode
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.encode().map_err(|source| {
            Repr::Encode {
                key: Key::Settings,
                source,
            }
            .into()
        })
    }

    /// Return the effective schedule mode.
    pub fn mode(&self) -> ScheduleMode {
        match (self.schedule_enabled, self.schedule_kind) {
            (false, _) => ScheduleMode::Off,
            (true, ScheduleKind::SunsetToSunrise) => ScheduleMode::SunsetToSunrise,
            (true, ScheduleKind::SetHours) => ScheduleMode::SetHours,
        }
    }

    /// Return whether the schedule is on.
    pub fn is_schedule_enabled(&self) -> bool {
        self.schedule_enabled
    }

    /// Return the schedule kind, which is kept while the schedule is off.
    pub fn schedule_kind(&self) -> ScheduleKind {
        self.schedule_kind
    }

    /// Return the time a set-hours schedule turns Night Light on.
    pub fn schedule_start(&self) -> TimeOfDay {
        self.schedule_start
    }

    /// Return the time a set-hours schedule turns Night Light off.
    pub fn schedule_end(&self) -> TimeOfDay {
        self.schedule_end
    }

    /// Return the stored color temperature in kelvin, which may be outside
    /// the range [`ColorTemperature`] accepts, or `None` if no value is stored
    /// and Windows applies its default.
    pub fn color_temperature(&self) -> Option<i16> {
        self.color_temperature
    }

    /// Return the sunset time Windows computed for the current location.
    pub fn sunset(&self) -> TimeOfDay {
        self.sunset
    }

    /// Return the sunrise time Windows computed for the current location.
    pub fn sunrise(&self) -> TimeOfDay {
        self.sunrise
    }

    /// Return whether the Settings app is previewing a color temperature.
    pub fn is_previewing(&self) -> bool {
        self.previewing
    }

    /// Return when the settings were last written, or `None` if the stored
    /// time is out of range.
    pub fn modified(&self) -> Option<SystemTime> {
        system_time_from_unix_secs(self.envelope.modified())
    }

    /// Change the schedule.
    pub fn set_schedule(&mut self, schedule: Schedule) {
        match schedule {
            Schedule::Off => self.schedule_enabled = false,
            Schedule::SunsetToSunrise => {
                self.schedule_enabled = true;
                self.schedule_kind = ScheduleKind::SunsetToSunrise;
            }
            Schedule::SetHours { start, end } => {
                self.schedule_enabled = true;
                self.schedule_kind = ScheduleKind::SetHours;
                self.schedule_start = start.unwrap_or(self.schedule_start);
                self.schedule_end = end.unwrap_or(self.schedule_end);
            }
        }
    }

    /// Change the color temperature.
    pub fn set_color_temperature(&mut self, temperature: ColorTemperature) {
        self.color_temperature = Some(temperature.kelvin().cast_signed());
    }

    pub(crate) fn stamp(&mut self, now: SystemTime) {
        self.envelope.stamp(now);
    }

    fn decode(data: &[u8]) -> Result<Self, BondError> {
        let (envelope, payload) = Envelope::decode(data)?;
        let mut reader = CompactBinaryReader::new(payload);
        reader.read_marshaled_header()?;

        let mut schedule_enabled = false;
        let mut schedule_kind = ScheduleKind::SunsetToSunrise;
        let mut schedule_start = TimeOfDay::default();
        let mut schedule_end = TimeOfDay::default();
        let mut sunset = TimeOfDay::default();
        let mut sunrise = TimeOfDay::default();
        let mut color_temperature = None;
        let mut previewing = false;
        let mut unknown = UnknownFields::default();
        reader.read_fields(|reader, id, bond_type| match id {
            SCHEDULE_ENABLED => {
                expect_type(id, bond_type, BondType::Bool)?;
                schedule_enabled = reader.read_bool()?;
                Ok(())
            }
            ON_SUN_SCHEDULE => {
                expect_type(id, bond_type, BondType::Bool)?;
                schedule_kind = if reader.read_bool()? {
                    ScheduleKind::SunsetToSunrise
                } else {
                    ScheduleKind::SetHours
                };
                Ok(())
            }
            SCHEDULE_START => {
                schedule_start = read_time_block(reader, id, bond_type)?;
                Ok(())
            }
            SCHEDULE_END => {
                schedule_end = read_time_block(reader, id, bond_type)?;
                Ok(())
            }
            SUNSET => {
                sunset = read_time_block(reader, id, bond_type)?;
                Ok(())
            }
            SUNRISE => {
                sunrise = read_time_block(reader, id, bond_type)?;
                Ok(())
            }
            COLOR_TEMPERATURE => {
                expect_type(id, bond_type, BondType::Int16)?;
                color_temperature = Some(reader.read_int16()?);
                Ok(())
            }
            PREVIEWING => {
                expect_type(id, bond_type, BondType::Bool)?;
                previewing = reader.read_bool()?;
                Ok(())
            }
            _ => unknown.capture(reader, id, bond_type),
        })?;
        reader.expect_end()?;

        Ok(Self {
            envelope,
            schedule_enabled,
            schedule_kind,
            schedule_start,
            schedule_end,
            color_temperature,
            sunset,
            sunrise,
            previewing,
            unknown,
        })
    }

    fn encode(&self) -> Result<Vec<u8>, BondError> {
        let mut writer = CompactBinaryWriter::new();
        writer.write_marshaled_header();
        writer.write_struct(&self.unknown, |fields| {
            if self.schedule_enabled {
                fields.bool(SCHEDULE_ENABLED, true);
            }
            if self.schedule_kind == ScheduleKind::SetHours {
                fields.bool(ON_SUN_SCHEDULE, false);
            }
            write_time_block(fields, SCHEDULE_START, self.schedule_start);
            write_time_block(fields, SCHEDULE_END, self.schedule_end);
            if let Some(kelvin) = self.color_temperature {
                fields.int16(COLOR_TEMPERATURE, kelvin);
            }
            write_time_block(fields, SUNSET, self.sunset);
            write_time_block(fields, SUNRISE, self.sunrise);
            if self.previewing {
                fields.bool(PREVIEWING, true);
            }
        });
        self.envelope.encode(&writer.into_bytes())
    }
}

fn read_time_block(
    reader: &mut CompactBinaryReader<'_>,
    id: u16,
    bond_type: BondType,
) -> Result<TimeOfDay, BondError> {
    expect_type(id, bond_type, BondType::Struct)?;
    let mut hour: i8 = 0;
    let mut minute: i8 = 0;
    // Unknown fields inside a time block are skipped and not written back;
    // the known schema defines only the hour and minute.
    reader.read_fields(|reader, field_id, bond_type| match field_id {
        TIME_HOUR => {
            expect_type(field_id, bond_type, BondType::Int8)?;
            hour = reader.read_int8()?;
            Ok(())
        }
        TIME_MINUTE => {
            expect_type(field_id, bond_type, BondType::Int8)?;
            minute = reader.read_int8()?;
            Ok(())
        }
        _ => reader.read_raw_value(bond_type).map(drop),
    })?;
    u8::try_from(hour)
        .ok()
        .zip(u8::try_from(minute).ok())
        .and_then(|(hour, minute)| TimeOfDay::from_parts(hour, minute))
        .ok_or_else(|| BondError::InvalidValue {
            id,
            value: format!("{hour}:{minute:02}"),
        })
}

fn write_time_block(fields: &mut StructWriter<'_, '_>, id: u16, time: TimeOfDay) {
    let hour = time.hour().cast_signed();
    let minute = time.minute().cast_signed();
    fields.field(id, BondType::Struct, |writer| {
        writer.write_struct(&UnknownFields::default(), |block| {
            if hour != 0 {
                block.int8(TIME_HOUR, hour);
            }
            if minute != 0 {
                block.int8(TIME_MINUTE, minute);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;
    use crate::fixtures;
    use proptest::prelude::*;
    use std::time::Duration;
    use std::time::UNIX_EPOCH;
    use test_case::test_case;

    fn decode(data: &[u8]) -> Settings {
        Settings::from_bytes(data).expect("fixture decodes")
    }

    fn time(hour: u8, minute: u8) -> TimeOfDay {
        TimeOfDay::new(hour, minute).expect("fixture time is in range")
    }

    #[test_case(fixtures::SETTINGS_SET_HOURS; "set hours")]
    #[test_case(fixtures::SETTINGS_LIVE; "live")]
    #[test_case(fixtures::SETTINGS_PREVIEWING; "previewing")]
    #[test_case(fixtures::SETTINGS_TEMPERATURE_OUT_OF_RANGE; "temperature out of range")]
    #[test_case(fixtures::SETTINGS_WITH_UNKNOWN_FIELDS; "unknown fields")]
    #[test_case(fixtures::SETTINGS_WITHOUT_TEMPERATURE; "without temperature")]
    fn round_trips_bytes(data: &[u8]) {
        assert_eq!(decode(data).to_bytes().expect("settings encode"), data);
    }

    #[test]
    fn decodes_set_hours_fixture() {
        let settings = decode(fixtures::SETTINGS_SET_HOURS);
        assert_eq!(settings.mode(), ScheduleMode::SetHours);
        assert!(settings.is_schedule_enabled());
        assert_eq!(settings.schedule_kind(), ScheduleKind::SetHours);
        assert_eq!(settings.schedule_start(), time(1, 15));
        assert_eq!(settings.schedule_end(), time(0, 0));
        assert_eq!(settings.color_temperature(), Some(2790));
        assert_eq!(settings.sunset(), time(19, 23));
        assert_eq!(settings.sunrise(), time(7, 12));
        assert!(!settings.is_previewing());
        assert_eq!(
            settings.modified(),
            Some(UNIX_EPOCH + Duration::from_secs(1_742_540_908))
        );
    }

    #[test]
    fn decodes_live_fixture() {
        let settings = decode(fixtures::SETTINGS_LIVE);
        assert_eq!(settings.mode(), ScheduleMode::Off);
        assert_eq!(settings.schedule_kind(), ScheduleKind::SunsetToSunrise);
        assert_eq!(settings.schedule_start(), time(21, 0));
        assert_eq!(settings.schedule_end(), time(7, 0));
        assert_eq!(settings.color_temperature(), Some(3426));
        assert_eq!(settings.sunset(), time(19, 17));
        assert_eq!(settings.sunrise(), time(6, 38));
        assert_eq!(
            settings.modified(),
            Some(UNIX_EPOCH + Duration::from_secs(1_791_093_359))
        );
    }

    #[test]
    fn decodes_previewing_flag() {
        assert!(decode(fixtures::SETTINGS_PREVIEWING).is_previewing());
    }

    #[test]
    fn decodes_settings_without_temperature() {
        let settings = decode(fixtures::SETTINGS_WITHOUT_TEMPERATURE);
        assert_eq!(settings.color_temperature(), None);
        assert_eq!(settings.mode(), ScheduleMode::Off);
        assert_eq!(settings.schedule_start(), time(21, 0));
        assert_eq!(settings.schedule_end(), time(7, 0));
        assert_eq!(settings.sunset(), time(0, 0));
        assert_eq!(
            settings.modified(),
            Some(UNIX_EPOCH + Duration::from_secs(1_597_959_528))
        );
    }

    #[test]
    fn setting_temperature_adds_missing_field() {
        let mut settings = decode(fixtures::SETTINGS_WITHOUT_TEMPERATURE);
        settings.set_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"));
        assert_eq!(
            settings.to_bytes().expect("settings encode"),
            fixtures::SETTINGS_WITHOUT_TEMPERATURE_SET_TO_3400K
        );
    }

    #[test]
    fn keeps_out_of_range_stored_temperature() {
        assert_eq!(
            decode(fixtures::SETTINGS_TEMPERATURE_OUT_OF_RANGE).color_temperature(),
            Some(7000)
        );
    }

    #[test]
    fn setter_change_keeps_unknown_fields_in_order() {
        let mut settings = decode(fixtures::SETTINGS_WITH_UNKNOWN_FIELDS);
        settings.set_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"));
        assert_eq!(
            settings.to_bytes().expect("settings encode"),
            fixtures::SETTINGS_WITH_UNKNOWN_FIELDS_3400K
        );
    }

    #[test_case(fixtures::SETTINGS_TEMPERATURE_AS_INT32, "field 40 has type Int32, expected Int16"; "inner field")]
    #[test_case(fixtures::SETTINGS_MODIFIED_AS_INT64, "field 0 has type Int64, expected UInt64"; "envelope field")]
    #[test_case(fixtures::SETTINGS_START_HOUR_25, "field 20 has invalid value 25:15"; "time out of range")]
    fn rejects_invalid_payload(data: &[u8], cause: &str) {
        let error = Settings::from_bytes(data).expect_err("payload is invalid");
        assert_eq!(error.kind(), ErrorKind::Decode);
        assert_eq!(error.to_string(), "decoding Night Light settings");
        let source = std::error::Error::source(&error).expect("decode error has a cause");
        assert_eq!(source.to_string(), cause);
    }

    #[test]
    fn schedule_off_keeps_kind_and_times() {
        let mut settings = decode(fixtures::SETTINGS_SET_HOURS);
        settings.set_schedule(Schedule::Off);
        assert_eq!(settings.mode(), ScheduleMode::Off);
        assert_eq!(settings.schedule_kind(), ScheduleKind::SetHours);
        assert_eq!(settings.schedule_start(), time(1, 15));
    }

    #[test]
    fn set_hours_keeps_unspecified_time() {
        let mut settings = decode(fixtures::SETTINGS_LIVE);
        settings.set_schedule(Schedule::SetHours {
            start: Some(time(22, 30)),
            end: None,
        });
        assert_eq!(settings.mode(), ScheduleMode::SetHours);
        assert_eq!(settings.schedule_start(), time(22, 30));
        assert_eq!(settings.schedule_end(), time(7, 0));
    }

    #[test]
    fn sunset_schedule_omits_on_sun_schedule_field() {
        let mut settings = decode(fixtures::SETTINGS_SET_HOURS);
        settings.set_schedule(Schedule::SunsetToSunrise);
        let bytes = settings.to_bytes().expect("settings encode");
        assert!(!bytes.windows(2).any(|pair| pair == [0xC2, 0x0A]));
        assert_eq!(decode(&bytes).mode(), ScheduleMode::SunsetToSunrise);
    }

    #[test_case(1199, false)]
    #[test_case(1200, true)]
    #[test_case(6500, true)]
    #[test_case(6501, false)]
    fn validates_color_temperature_range(kelvin: u16, valid: bool) {
        assert_eq!(ColorTemperature::new(kelvin).is_ok(), valid);
        assert_eq!(ColorTemperature::try_from(kelvin).is_ok(), valid);
    }

    #[test_case("3400", 3400)]
    #[test_case("3400K", 3400)]
    #[test_case("3400k", 3400)]
    #[test_case("1200", 1200)]
    fn parses_color_temperature(text: &str, kelvin: u16) {
        let temperature = text
            .parse::<ColorTemperature>()
            .expect("text is a valid temperature");
        assert_eq!(temperature.kelvin(), kelvin);
    }

    #[test_case("7000", "color temperature 7000K is outside 1200K-6500K")]
    #[test_case("70000", "color temperature 70000K is outside 1200K-6500K")]
    #[test_case(
        "warm",
        "invalid color temperature \"warm\", expected kelvin from 1200 to 6500"
    )]
    #[test_case(
        "",
        "invalid color temperature \"\", expected kelvin from 1200 to 6500"
    )]
    #[test_case(
        "-3400",
        "invalid color temperature \"-3400\", expected kelvin from 1200 to 6500"
    )]
    fn rejects_color_temperature(text: &str, message: &str) {
        let error = text
            .parse::<ColorTemperature>()
            .expect_err("text is not a valid temperature");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), message);
    }

    #[test]
    fn displays_color_temperature_with_unit() {
        assert_eq!(ColorTemperature::MIN.to_string(), "1200K");
    }

    fn any_time() -> impl Strategy<Value = TimeOfDay> {
        (0..24_u8, 0..60_u8).prop_map(|(hour, minute)| time(hour, minute))
    }

    fn any_schedule() -> impl Strategy<Value = Schedule> {
        prop_oneof![
            Just(Schedule::Off),
            Just(Schedule::SunsetToSunrise),
            (
                proptest::option::of(any_time()),
                proptest::option::of(any_time())
            )
                .prop_map(|(start, end)| Schedule::SetHours { start, end }),
        ]
    }

    proptest! {
        #[test]
        fn setter_changes_round_trip(
            schedule in any_schedule(),
            kelvin in ColorTemperature::MIN.kelvin()..=ColorTemperature::MAX.kelvin(),
        ) {
            let temperature = ColorTemperature::new(kelvin).expect("strategy yields valid kelvin");
            let mut settings = decode(fixtures::SETTINGS_WITH_UNKNOWN_FIELDS);
            settings.set_schedule(schedule);
            settings.set_color_temperature(temperature);

            let decoded = decode(&settings.to_bytes().expect("settings encode"));
            prop_assert_eq!(&decoded, &settings);
            prop_assert_eq!(decoded.color_temperature(), Some(kelvin.cast_signed()));
            if let Schedule::SetHours { start: Some(start), end: Some(end) } = schedule {
                prop_assert_eq!(decoded.mode(), ScheduleMode::SetHours);
                prop_assert_eq!((decoded.schedule_start(), decoded.schedule_end()), (start, end));
            }
        }
    }
}
