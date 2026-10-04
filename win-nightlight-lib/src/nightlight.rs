use crate::Result;
use crate::Settings;
use crate::State;
use crate::error::Repr;
use crate::store::Key;
use crate::store::RegistryStore;
use crate::store::Store;
use std::fmt;
use std::time::SystemTime;

const READ_ATTEMPTS: usize = 3;

/// Handle to the current user's Night Light settings and state.
///
/// Settings (schedule and color temperature) and state (whether Night Light
/// is on) are separate registry values; each method reads or writes only
/// the value it names.
pub struct Nightlight {
    store: Box<dyn Store + Send + Sync>,
    clock: fn() -> SystemTime,
}

impl Nightlight {
    /// Create a handle backed by the current user's registry.
    pub fn new() -> Self {
        Self::with_store(RegistryStore, SystemTime::now)
    }

    pub(crate) fn with_store(
        store: impl Store + Send + Sync + 'static,
        clock: fn() -> SystemTime,
    ) -> Self {
        Self {
            store: Box::new(store),
            clock,
        }
    }

    /// Read the current settings.
    ///
    /// # Errors
    ///
    /// Returns an error of kind:
    ///
    /// - [`ErrorKind::NotFound`] if the settings value does not exist.
    /// - [`ErrorKind::Registry`] if the registry read fails.
    /// - [`ErrorKind::Decode`] if the value is not a valid settings payload.
    ///
    /// [`ErrorKind::NotFound`]: crate::ErrorKind::NotFound
    /// [`ErrorKind::Registry`]: crate::ErrorKind::Registry
    /// [`ErrorKind::Decode`]: crate::ErrorKind::Decode
    pub fn read_settings(&self) -> Result<Settings> {
        self.read_decoded(Key::Settings, Settings::from_bytes)
    }

    /// Write `settings` with a new last-modified time.
    ///
    /// The new last-modified time is the current time, or two seconds after
    /// the time stored in `settings` if that is later, because Windows
    /// ignores a write whose time is older than the stored time.
    ///
    /// # Errors
    ///
    /// Returns an error of kind:
    ///
    /// - [`ErrorKind::Encode`] if `settings` does not fit the payload format.
    /// - [`ErrorKind::NotFound`] if the settings registry key does not exist.
    /// - [`ErrorKind::Registry`] if the registry write fails.
    ///
    /// [`ErrorKind::Encode`]: crate::ErrorKind::Encode
    /// [`ErrorKind::NotFound`]: crate::ErrorKind::NotFound
    /// [`ErrorKind::Registry`]: crate::ErrorKind::Registry
    pub fn write_settings(&self, settings: &Settings) -> Result<()> {
        let mut settings = settings.clone();
        settings.stamp((self.clock)());
        self.write(Key::Settings, &settings.to_bytes()?)
    }

    /// Read the settings, apply `change`, and write them back if `change`
    /// modified them.
    ///
    /// Returns whether it wrote the settings.
    ///
    /// # Errors
    ///
    /// Returns any error of [`read_settings`](Self::read_settings) or
    /// [`write_settings`](Self::write_settings).
    pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) -> Result<bool> {
        let current = self.read_settings()?;
        let mut updated = current.clone();
        change(&mut updated);
        if updated == current {
            return Ok(false);
        }
        self.write_settings(&updated)?;
        Ok(true)
    }

    /// Read the current state.
    ///
    /// # Errors
    ///
    /// Returns an error of kind:
    ///
    /// - [`ErrorKind::NotFound`] if the state value does not exist.
    /// - [`ErrorKind::Registry`] if the registry read fails.
    /// - [`ErrorKind::Decode`] if the value is not a valid state payload.
    ///
    /// [`ErrorKind::NotFound`]: crate::ErrorKind::NotFound
    /// [`ErrorKind::Registry`]: crate::ErrorKind::Registry
    /// [`ErrorKind::Decode`]: crate::ErrorKind::Decode
    pub fn read_state(&self) -> Result<State> {
        self.read_decoded(Key::State, State::from_bytes)
    }

    /// Write `state` with a new last-modified time.
    ///
    /// The last-modified time follows the same rule as
    /// [`write_settings`](Self::write_settings). This does not record a
    /// transition; use [`update_state`](Self::update_state) to turn Night
    /// Light on or off.
    ///
    /// # Errors
    ///
    /// Returns an error of kind:
    ///
    /// - [`ErrorKind::Encode`] if `state` does not fit the payload format.
    /// - [`ErrorKind::NotFound`] if the state registry key does not exist.
    /// - [`ErrorKind::Registry`] if the registry write fails.
    ///
    /// [`ErrorKind::Encode`]: crate::ErrorKind::Encode
    /// [`ErrorKind::NotFound`]: crate::ErrorKind::NotFound
    /// [`ErrorKind::Registry`]: crate::ErrorKind::Registry
    pub fn write_state(&self, state: &State) -> Result<()> {
        self.write_state_at(state, (self.clock)())
    }

    /// Read the state, apply `change`, and write it back if `change` modified
    /// it.
    ///
    /// When `change` turns Night Light on or off, this records a manual
    /// transition at the current time. Returns whether it wrote the state.
    ///
    /// # Errors
    ///
    /// Returns any error of [`read_state`](Self::read_state) or
    /// [`write_state`](Self::write_state).
    pub fn update_state(&self, change: impl FnOnce(&mut State)) -> Result<bool> {
        let current = self.read_state()?;
        let mut updated = current.clone();
        change(&mut updated);
        if updated == current {
            return Ok(false);
        }
        let now = (self.clock)();
        if updated.is_active() != current.is_active() {
            updated.record_manual_transition(now);
        }
        self.write_state_at(&updated, now)?;
        Ok(true)
    }

    /// Turn Night Light on or off until the next scheduled transition.
    ///
    /// Returns whether it wrote the state; it does not write when Night Light
    /// is already in the requested state.
    ///
    /// # Errors
    ///
    /// Returns any error of [`update_state`](Self::update_state).
    pub fn set_active(&self, active: bool) -> Result<bool> {
        self.update_state(|state| state.set_active(active))
    }

    /// Turn Night Light on if it is off, or off if it is on, until the next
    /// scheduled transition.
    ///
    /// Returns whether Night Light is now on.
    ///
    /// # Errors
    ///
    /// Returns any error of [`update_state`](Self::update_state).
    pub fn toggle(&self) -> Result<bool> {
        let mut active = false;
        self.update_state(|state| {
            active = !state.is_active();
            state.set_active(active);
        })?;
        Ok(active)
    }

    fn write_state_at(&self, state: &State, now: SystemTime) -> Result<()> {
        let mut state = state.clone();
        state.stamp(now);
        self.write(Key::State, &state.to_bytes()?)
    }

    // `windows-registry` sizes its buffer with one query and fills it with a
    // second. If Windows rewrites the value in between, a longer value fails
    // with `ERROR_MORE_DATA` and a shorter one comes back padded with zeros,
    // which fails to decode. Both clear up on a fresh read.
    fn read_decoded<T>(&self, key: Key, decode: impl Fn(&[u8]) -> Result<T>) -> Result<T> {
        let mut attempt = 1;
        loop {
            match self.read(key).and_then(|data| decode(&data)) {
                Err(error) if attempt < READ_ATTEMPTS && error.may_be_torn_read() => attempt += 1,
                result => return result,
            }
        }
    }

    fn read(&self, key: Key) -> Result<Vec<u8>> {
        self.store
            .read(key)
            .map_err(|source| Repr::Read { key, source }.into())
    }

    fn write(&self, key: Key, data: &[u8]) -> Result<()> {
        self.store
            .write(key, data)
            .map_err(|source| Repr::Write { key, source }.into())
    }
}

impl Default for Nightlight {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Nightlight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Nightlight").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColorTemperature;
    use crate::ErrorKind;
    use crate::Schedule;
    use crate::ScheduleKind;
    use crate::ScheduleMode;
    use crate::TransitionCause;
    use crate::fixtures;
    use crate::store::memory::MemoryStore;
    use hex_literal::hex;
    use std::time::Duration;
    use std::time::UNIX_EPOCH;
    use test_case::test_case;
    use windows_result::WIN32_ERROR;

    const CLOCK_SECS: u64 = 1_800_000_000;

    fn clock() -> SystemTime {
        UNIX_EPOCH + Duration::new(CLOCK_SECS, 123_456_700)
    }

    fn clock_before_fixtures() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    fn nightlight(state: &[u8], clock: fn() -> SystemTime) -> (Nightlight, MemoryStore) {
        let store = MemoryStore::with(&[
            (Key::Settings, fixtures::SETTINGS_SET_HOURS),
            (Key::State, state),
        ]);
        (Nightlight::with_store(store.clone(), clock), store)
    }

    fn stored_state(store: &MemoryStore) -> State {
        let data = store.value(Key::State).expect("state value exists");
        State::from_bytes(&data).expect("written state decodes")
    }

    fn stored_settings(store: &MemoryStore) -> Settings {
        let data = store.value(Key::Settings).expect("settings value exists");
        Settings::from_bytes(&data).expect("written settings decode")
    }

    #[test]
    fn nightlight_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Nightlight>();
    }

    #[test]
    fn turning_on_writes_only_state_with_manual_transition() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock);
        assert!(nightlight.set_active(true).expect("state updates"));

        assert_eq!(store.writes(), [Key::State]);
        assert_eq!(
            store.value(Key::State).expect("state value exists"),
            hex!(
                "43420100 0A020100 2A 0680A4A7DA06 2A 2B0E15"
                "43420100 1000 D00A02 C61487AD99DAE698E9EE01 00"
                "00 00 00"
            )
        );
        assert_eq!(
            store.value(Key::Settings).as_deref(),
            Some(fixtures::SETTINGS_SET_HOURS)
        );
    }

    #[test_case(fixtures::STATE_ACTIVE, false; "turn off")]
    #[test_case(fixtures::STATE_ACTIVE_SCHEDULED, false; "turn off after scheduled transition")]
    #[test_case(fixtures::STATE_WITH_UNKNOWN_FIELD, true; "turn on with unknown field")]
    fn records_manual_transition(initial: &[u8], active: bool) {
        let (nightlight, store) = nightlight(initial, clock);
        assert!(nightlight.set_active(active).expect("state updates"));

        let state = stored_state(&store);
        assert_eq!(state.is_active(), active);
        assert_eq!(state.transition_cause(), TransitionCause::Manual);
        assert_eq!(state.last_transition(), Some(clock()));
        assert_eq!(
            state.modified(),
            Some(UNIX_EPOCH + Duration::from_secs(CLOCK_SECS))
        );
        assert_eq!(store.writes(), [Key::State]);
    }

    #[test]
    fn keeps_unknown_state_field() {
        let (nightlight, store) = nightlight(fixtures::STATE_WITH_UNKNOWN_FIELD, clock);
        nightlight.set_active(true).expect("state updates");
        let data = store.value(Key::State).expect("state value exists");
        assert!(data.windows(2).any(|pair| pair == [0xA2, 0x01]));
    }

    #[test]
    fn advances_modified_time_past_stored_time_when_clock_is_behind() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock_before_fixtures);
        nightlight.set_active(true).expect("state updates");
        assert_eq!(
            stored_state(&store).modified(),
            Some(UNIX_EPOCH + Duration::from_secs(1_742_670_473 + 2))
        );
    }

    #[test_case(fixtures::STATE_ACTIVE, true; "already on")]
    #[test_case(fixtures::STATE_INACTIVE, false; "already off")]
    fn does_not_write_unchanged_state(initial: &[u8], active: bool) {
        let (nightlight, store) = nightlight(initial, clock);
        assert!(!nightlight.set_active(active).expect("state reads"));
        assert_eq!(store.writes(), []);
    }

    #[test]
    fn toggle_flips_active_state() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock);
        assert!(nightlight.toggle().expect("state toggles on"));
        assert!(stored_state(&store).is_active());
        assert!(!nightlight.toggle().expect("state toggles off"));
        assert!(!stored_state(&store).is_active());
        assert_eq!(store.writes(), [Key::State, Key::State]);
    }

    #[test]
    fn color_temperature_change_writes_only_settings() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock);
        let temperature = ColorTemperature::new(3400).expect("3400K is in range");
        assert!(
            nightlight
                .update_settings(|settings| settings.set_color_temperature(temperature))
                .expect("settings update")
        );

        assert_eq!(store.writes(), [Key::Settings]);
        let settings = stored_settings(&store);
        assert_eq!(settings.color_temperature(), Some(3400));
        assert_eq!(
            settings.modified(),
            Some(UNIX_EPOCH + Duration::from_secs(CLOCK_SECS))
        );
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_INACTIVE)
        );
    }

    #[test]
    fn schedule_change_writes_only_settings() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock);
        assert!(
            nightlight
                .update_settings(|settings| settings.set_schedule(Schedule::SunsetToSunrise))
                .expect("settings update")
        );
        assert_eq!(store.writes(), [Key::Settings]);
        assert_eq!(
            stored_settings(&store).mode(),
            ScheduleMode::SunsetToSunrise
        );
    }

    #[test]
    fn schedule_off_keeps_kind() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock);
        nightlight
            .update_settings(|settings| settings.set_schedule(Schedule::Off))
            .expect("settings update");
        let settings = stored_settings(&store);
        assert_eq!(settings.mode(), ScheduleMode::Off);
        assert_eq!(settings.schedule_kind(), ScheduleKind::SetHours);
    }

    #[test]
    fn does_not_write_unchanged_settings() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock);
        let temperature = ColorTemperature::new(2790).expect("2790K is in range");
        assert!(
            !nightlight
                .update_settings(|settings| settings.set_color_temperature(temperature))
                .expect("settings read")
        );
        assert_eq!(store.writes(), []);
    }

    #[test]
    fn missing_value_is_not_found() {
        let nightlight = Nightlight::with_store(MemoryStore::default(), clock);
        let error = nightlight.read_state().expect_err("state value is missing");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(
            error.to_string(),
            "reading Night Light state from the registry"
        );
    }

    #[test]
    fn retries_read_when_value_grows_between_queries() {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_reads(Key::State, vec![Err(WIN32_ERROR(234))]);
        assert!(
            nightlight
                .read_state()
                .expect("second read succeeds")
                .is_active()
        );
        assert_eq!(store.reads(), [Key::State, Key::State]);
    }

    #[test]
    fn retries_read_when_value_shrinks_between_queries() {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        let padded = [fixtures::STATE_INACTIVE, &[0, 0]].concat();
        store.queue_reads(Key::State, vec![Ok(padded)]);
        assert!(
            nightlight
                .read_state()
                .expect("second read succeeds")
                .is_active()
        );
        assert_eq!(store.reads(), [Key::State, Key::State]);
    }

    #[test]
    fn stops_retrying_a_payload_that_never_decodes() {
        let store = MemoryStore::with(&[(Key::Settings, &[0x43, 0x42, 0x01])]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        let error = nightlight
            .read_settings()
            .expect_err("payload is truncated");
        assert_eq!(error.kind(), ErrorKind::Decode);
        assert_eq!(store.reads(), [Key::Settings; READ_ATTEMPTS]);
    }

    #[test]
    fn does_not_retry_missing_value() {
        let store = MemoryStore::default();
        let nightlight = Nightlight::with_store(store.clone(), clock);
        nightlight.read_state().expect_err("state value is missing");
        assert_eq!(store.reads(), [Key::State]);
    }
}
