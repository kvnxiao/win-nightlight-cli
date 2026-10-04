use crate::Error;
use crate::Result;
use crate::error::Repr;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const FILETIME_TICKS_PER_SEC: u64 = 10_000_000;
const NANOS_PER_FILETIME_TICK: u32 = 100;
const FILETIME_TO_UNIX_EPOCH: Duration = Duration::from_hours(3_234_576);

/// Wall-clock time of day with minute precision, from 00:00 to 23:59.
///
/// Parses from and displays as 24-hour `HH:MM`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay {
    hour: u8,
    minute: u8,
}

impl TimeOfDay {
    /// Create a time of day from a 24-hour `hour` and a `minute`.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`ErrorKind::InvalidInput`] if `hour` is
    /// greater than 23 or `minute` is greater than 59.
    ///
    /// [`ErrorKind::InvalidInput`]: crate::ErrorKind::InvalidInput
    pub fn new(hour: u8, minute: u8) -> Result<Self> {
        Self::from_parts(hour, minute).ok_or_else(|| Repr::TimeOfDayRange { hour, minute }.into())
    }

    pub(crate) fn from_parts(hour: u8, minute: u8) -> Option<Self> {
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }

    /// Return the hour, from 0 to 23.
    pub fn hour(self) -> u8 {
        self.hour
    }

    /// Return the minute, from 0 to 59.
    pub fn minute(self) -> u8 {
        self.minute
    }

    /// Return whether this time falls in the window that starts at `start`
    /// (inclusive) and ends at `end` (exclusive).
    ///
    /// A window whose `end` is earlier than its `start` wraps past midnight.
    /// A window whose `start` equals its `end` is empty.
    pub fn is_within(self, start: Self, end: Self) -> bool {
        if start <= end {
            start <= self && self < end
        } else {
            start <= self || self < end
        }
    }
}

impl FromStr for TimeOfDay {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let parse = |digits: &str, max_len: usize| {
            (!digits.is_empty()
                && digits.len() <= max_len
                && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| digits.parse::<u8>().ok())
            .flatten()
        };
        text.split_once(':')
            .filter(|(_, minute)| minute.len() == 2)
            .and_then(|(hour, minute)| Some((parse(hour, 2)?, parse(minute, 2)?)))
            .and_then(|(hour, minute)| Self::from_parts(hour, minute))
            .ok_or_else(|| Repr::TimeOfDaySyntax(text.to_owned()).into())
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

pub(crate) fn system_time_from_filetime(filetime: u64) -> Option<SystemTime> {
    let nanos = u32::try_from(filetime % FILETIME_TICKS_PER_SEC).ok()? * NANOS_PER_FILETIME_TICK;
    let since_1601 = Duration::new(filetime / FILETIME_TICKS_PER_SEC, nanos);
    match since_1601.checked_sub(FILETIME_TO_UNIX_EPOCH) {
        Some(after_unix_epoch) => UNIX_EPOCH.checked_add(after_unix_epoch),
        None => UNIX_EPOCH.checked_sub(FILETIME_TO_UNIX_EPOCH.saturating_sub(since_1601)),
    }
}

/// Convert `time` to a FILETIME, saturating at the ends of the FILETIME
/// range.
pub(crate) fn filetime_from_system_time(time: SystemTime) -> u64 {
    let epoch_ticks = filetime_ticks(FILETIME_TO_UNIX_EPOCH);
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => epoch_ticks.saturating_add(filetime_ticks(after)),
        Err(before) => epoch_ticks.saturating_sub(filetime_ticks(before.duration())),
    }
}

fn filetime_ticks(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_mul(FILETIME_TICKS_PER_SEC)
        .saturating_add(u64::from(duration.subsec_nanos() / NANOS_PER_FILETIME_TICK))
}

pub(crate) fn system_time_from_unix_secs(secs: u64) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}

/// Convert `time` to whole Unix seconds, clamping times before 1970 to zero.
pub(crate) fn unix_secs_from_system_time(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;
    use proptest::prelude::*;
    use test_case::test_case;

    #[test_case("00:00", 0, 0)]
    #[test_case("07:05", 7, 5)]
    #[test_case("7:05", 7, 5)]
    #[test_case("21:00", 21, 0)]
    #[test_case("23:59", 23, 59)]
    fn parses_time_of_day(text: &str, hour: u8, minute: u8) {
        let time = text.parse::<TimeOfDay>().expect("text is a valid time");
        assert_eq!((time.hour(), time.minute()), (hour, minute));
    }

    #[test_case(""; "empty")]
    #[test_case("24:00"; "hour out of range")]
    #[test_case("12:60"; "minute out of range")]
    #[test_case("12:5"; "one digit minute")]
    #[test_case("012:00"; "three digit hour")]
    #[test_case(":30"; "missing hour")]
    #[test_case("+1:30"; "signed hour")]
    #[test_case("12:30:00"; "seconds")]
    #[test_case("1230"; "missing colon")]
    #[test_case(" 12:30"; "leading space")]
    #[test_case("9pm"; "twelve hour")]
    fn rejects_time_of_day(text: &str) {
        let error = text
            .parse::<TimeOfDay>()
            .expect_err("text is not a valid time");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert_eq!(
            error.to_string(),
            format!("invalid time of day {text:?}, expected HH:MM")
        );
    }

    #[test]
    fn displays_time_of_day_as_hh_mm() {
        let time = TimeOfDay::new(7, 5).expect("time is in range");
        assert_eq!(time.to_string(), "07:05");
    }

    #[test_case(24, 0)]
    #[test_case(0, 60)]
    fn rejects_out_of_range_parts(hour: u8, minute: u8) {
        let error = TimeOfDay::new(hour, minute).expect_err("parts are out of range");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    fn time(text: &str) -> TimeOfDay {
        text.parse().expect("fixture time is valid")
    }

    #[test_case("12:00", "09:00", "17:00", true; "inside day window")]
    #[test_case("09:00", "09:00", "17:00", true; "at day start")]
    #[test_case("17:00", "09:00", "17:00", false; "at day end")]
    #[test_case("08:59", "09:00", "17:00", false; "before day window")]
    #[test_case("23:00", "21:00", "07:00", true; "before midnight in wrapped window")]
    #[test_case("03:00", "21:00", "07:00", true; "after midnight in wrapped window")]
    #[test_case("07:00", "21:00", "07:00", false; "at wrapped end")]
    #[test_case("12:00", "21:00", "07:00", false; "outside wrapped window")]
    #[test_case("12:00", "12:00", "12:00", false; "empty window")]
    fn tests_window_membership(now: &str, start: &str, end: &str, expected: bool) {
        assert_eq!(time(now).is_within(time(start), time(end)), expected);
    }

    #[test]
    fn converts_filetime_to_system_time() {
        let expected = UNIX_EPOCH + Duration::new(1_742_667_580, 927_056_900);
        assert_eq!(
            system_time_from_filetime(133_871_411_809_270_569),
            Some(expected)
        );
        assert_eq!(filetime_from_system_time(expected), 133_871_411_809_270_569);
    }

    #[test]
    fn converts_unix_epoch_filetime() {
        assert_eq!(
            system_time_from_filetime(116_444_736_000_000_000),
            Some(UNIX_EPOCH)
        );
        assert_eq!(
            filetime_from_system_time(UNIX_EPOCH),
            116_444_736_000_000_000
        );
    }

    #[test]
    fn truncates_sub_tick_nanoseconds() {
        let time = UNIX_EPOCH + Duration::new(1, 199);
        assert_eq!(filetime_from_system_time(time), 116_444_736_010_000_001);
    }

    #[test]
    fn converts_unix_seconds() {
        assert_eq!(
            system_time_from_unix_secs(1_742_540_908),
            Some(UNIX_EPOCH + Duration::from_secs(1_742_540_908))
        );
        assert_eq!(
            unix_secs_from_system_time(UNIX_EPOCH + Duration::new(5, 999_999_999)),
            5
        );
    }

    proptest! {
        #[test]
        fn filetime_round_trips(filetime in 0..=i64::MAX.cast_unsigned()) {
            let time = system_time_from_filetime(filetime).expect("FILETIME up to i64::MAX is representable");
            prop_assert_eq!(filetime_from_system_time(time), filetime);
        }
    }
}
