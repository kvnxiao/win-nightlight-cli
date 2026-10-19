use crate::ColorTemperature;
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

/// Select when a temperature update requests a preview refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshPolicy {
    /// Preserve existing preview; otherwise refresh only while Night Light is
    /// active.
    Automatic,
    /// Request preview on and off on every call, regardless of active state or
    /// existing preview.
    Always,
}

/// Handle to the current user's Night Light settings and state.
///
/// Settings (schedule and color temperature) and state (whether Night Light
/// is on) are separate registry values. Temperature updates can read both
/// values and write only settings; on/off updates write only state.
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
    /// the time stored in `settings` if that is later. The time never moves
    /// backward, because `explorer.exe` does not apply settings stamped at or
    /// below the highest time it has applied.
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

    /// Set the color temperature with the selected preview refresh policy.
    ///
    /// - With [`RefreshPolicy::Automatic`], preserve existing preview;
    ///   otherwise refresh while active, or only store a changed temperature
    ///   while inactive.
    /// - With [`RefreshPolicy::Always`], write the temperature with preview on,
    ///   then reread settings and clear preview, even when both values already
    ///   match. Do not read or write the on/off state.
    /// - Refresh without a sleep. Each write advances the supplied record's
    ///   timestamp by at least two seconds unless it saturates at `u64::MAX`.
    /// - Return whether settings were written; a successful `Always` call
    ///   returns `true`. Neither policy writes the on/off state.
    /// - Cleanup preserves intervening settings changes, including temperature.
    ///   Writes are not atomic with other writers; avoid overlapping calls and
    ///   native slider interaction.
    /// - Successful writes do not acknowledge a display update. Windows may
    ///   combine notifications for the two writes.
    ///
    /// # Errors
    ///
    /// - Return any error of [`read_settings`](Self::read_settings),
    ///   [`read_state`](Self::read_state), or
    ///   [`write_settings`](Self::write_settings).
    /// - Return [`ErrorKind::PreviewCleanup`] if three attempts to clear
    ///   preview after writing the temperature fail. Preview may remain active;
    ///   the source records the last cleanup error. A later `Always` call
    ///   repeats the complete refresh.
    ///
    /// [`ErrorKind::PreviewCleanup`]: crate::ErrorKind::PreviewCleanup
    pub fn set_color_temperature(
        &self,
        temperature: ColorTemperature,
        refresh: RefreshPolicy,
    ) -> Result<bool> {
        let mut settings = self.read_settings()?;
        let changed = settings.color_temperature() != Some(temperature.kelvin().cast_signed());
        let start_preview = match refresh {
            RefreshPolicy::Automatic => !settings.is_previewing() && self.read_state()?.is_active(),
            RefreshPolicy::Always => true,
        };
        if !changed && !start_preview {
            return Ok(false);
        }
        settings.set_color_temperature(temperature);
        if start_preview {
            settings.set_previewing(true);
        }
        self.write_settings(&settings)?;
        if !start_preview {
            return Ok(true);
        }
        let mut attempts_left = 3;
        loop {
            attempts_left -= 1;
            match self.update_settings(|settings| settings.set_previewing(false)) {
                Ok(_) => return Ok(true),
                Err(source) if attempts_left == 0 => {
                    return Err(Repr::PreviewCleanup {
                        source: Box::new(source),
                    }
                    .into());
                }
                Err(_) => {}
            }
        }
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

    #[test_case(fixtures::STATE_ACTIVE; "manual activation")]
    #[test_case(fixtures::STATE_ACTIVE_SCHEDULED; "scheduled activation")]
    fn active_temperature_update_writes_preview_then_clears_it(state: &[u8]) {
        let (nightlight, store) = nightlight(state, clock);
        assert!(
            nightlight
                .set_color_temperature(
                    "3400".parse().expect("valid temperature"),
                    RefreshPolicy::Automatic
                )
                .expect("temperature updates")
        );
        assert_eq!(store.write_attempts(), vec![
            (Key::Settings, hex!(
                "43420100 0A020100 2A 0680A4A7DA06 2A 2B0E29"
                "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF289035 CA320E132E1700 CA3C0E072E0C00 C24601 00"
                "00 00 00"
            ).to_vec()),
            (Key::Settings, hex!(
                "43420100 0A020100 2A 0682A4A7DA06 2A 2B0E26"
                "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF289035 CA320E132E1700 CA3C0E072E0C00 00"
                "00 00 00"
            ).to_vec()),
        ]);
        assert_eq!(store.value(Key::State).as_deref(), Some(state));
        assert_eq!(store.reads(), [Key::Settings, Key::State, Key::Settings]);
    }

    #[test_case(fixtures::SETTINGS_SET_HOURS; "schedule enabled")]
    #[test_case(fixtures::SETTINGS_LIVE; "schedule off")]
    fn inactive_temperature_update_does_not_start_preview(initial: &[u8]) {
        let store = MemoryStore::with(&[
            (Key::Settings, initial),
            (Key::State, fixtures::STATE_INACTIVE),
        ]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        assert!(
            nightlight
                .set_color_temperature(
                    "3400".parse().expect("valid temperature"),
                    RefreshPolicy::Automatic
                )
                .expect("temperature updates")
        );
        assert_eq!(store.writes(), [Key::Settings]);
        let settings = stored_settings(&store);
        assert_eq!(settings.color_temperature(), Some(3400));
        assert!(!settings.is_previewing());
        assert_eq!(
            settings.mode(),
            Settings::from_bytes(initial)
                .expect("initial settings decode")
                .mode()
        );
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_INACTIVE)
        );
    }

    #[test]
    fn unchanged_inactive_temperature_does_not_write() {
        let (nightlight, store) = nightlight(fixtures::STATE_INACTIVE, clock);
        assert!(
            !nightlight
                .set_color_temperature(
                    "2790".parse().expect("valid temperature"),
                    RefreshPolicy::Automatic
                )
                .expect("settings read")
        );
        assert_eq!(store.writes(), []);
    }

    #[test_case(fixtures::SETTINGS_PREVIEWING, fixtures::STATE_ACTIVE, 2790; "preview on active same")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, fixtures::STATE_ACTIVE, 3400; "preview on active changed")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, fixtures::STATE_INACTIVE, 2790; "preview on inactive same")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, fixtures::STATE_INACTIVE, 3400; "preview on inactive changed")]
    #[test_case(fixtures::SETTINGS_SET_HOURS, fixtures::STATE_ACTIVE, 2790; "preview off active same")]
    #[test_case(fixtures::SETTINGS_SET_HOURS, fixtures::STATE_ACTIVE, 3400; "preview off active changed")]
    #[test_case(fixtures::SETTINGS_SET_HOURS, fixtures::STATE_INACTIVE, 2790; "preview off inactive same")]
    #[test_case(fixtures::SETTINGS_SET_HOURS, fixtures::STATE_INACTIVE, 3400; "preview off inactive changed")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, fixtures::STATE_INACTIVE, 1200; "minimum temperature")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, fixtures::STATE_INACTIVE, 6500; "maximum temperature")]
    fn always_refresh_writes_preview_on_then_off(initial: &[u8], state: &[u8], kelvin: u16) {
        let store = MemoryStore::with(&[(Key::Settings, initial), (Key::State, state)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        assert!(
            nightlight
                .set_color_temperature(
                    ColorTemperature::new(kelvin).expect("valid temperature"),
                    RefreshPolicy::Always
                )
                .expect("refresh completes")
        );
        assert!(!stored_settings(&store).is_previewing());
        assert_eq!(store.writes(), [Key::Settings, Key::Settings]);
        assert_eq!(store.reads(), [Key::Settings, Key::Settings]);
        assert_eq!(store.value(Key::State).as_deref(), Some(state));
        for ((_, bytes), previewing) in store.write_attempts().iter().zip([true, false]) {
            let settings = Settings::from_bytes(bytes).expect("written settings decode");
            assert_eq!(settings.color_temperature(), Some(kelvin.cast_signed()));
            assert_eq!(settings.is_previewing(), previewing);
            assert_eq!(settings.mode(), ScheduleMode::SetHours);
        }
    }

    #[test]
    fn always_refresh_does_not_require_state() {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_SET_HOURS)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        assert!(
            nightlight
                .set_color_temperature(
                    "2790".parse().expect("valid temperature"),
                    RefreshPolicy::Always
                )
                .expect("refresh needs only settings")
        );
        assert_eq!(store.reads(), [Key::Settings, Key::Settings]);
        assert_eq!(store.writes(), [Key::Settings, Key::Settings]);
        assert!(!stored_settings(&store).is_previewing());
        assert_eq!(store.value(Key::State), None);
    }

    #[test_case(false; "fixed clock")]
    #[test_case(true; "clock moves backward")]
    fn repeated_refresh_advances_timestamps_with_unchanged_temperature(clock_moves_backward: bool) {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_PREVIEWING)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        let temperature = "2790".parse().expect("valid temperature");
        assert!(
            nightlight
                .set_color_temperature(temperature, RefreshPolicy::Always)
                .expect("first refresh completes")
        );
        let next_clock = if clock_moves_backward {
            clock_before_fixtures
        } else {
            clock
        };
        let nightlight = Nightlight::with_store(store.clone(), next_clock);
        assert!(
            nightlight
                .set_color_temperature(temperature, RefreshPolicy::Always)
                .expect("second refresh completes")
        );
        assert_eq!(store.writes(), [Key::Settings; 4]);
        for ((_, bytes), (previewing, seconds)) in store.write_attempts().iter().zip([
            (true, 1_800_000_000),
            (false, 1_800_000_002),
            (true, 1_800_000_004),
            (false, 1_800_000_006),
        ]) {
            let settings = Settings::from_bytes(bytes).expect("written settings decode");
            assert_eq!(settings.is_previewing(), previewing);
            assert_eq!(settings.color_temperature(), Some(2790));
            assert_eq!(
                settings.modified(),
                Some(UNIX_EPOCH + Duration::from_secs(seconds))
            );
        }
    }

    #[test_case(RefreshPolicy::Automatic; "automatic")]
    #[test_case(RefreshPolicy::Always; "always")]
    fn temperature_update_aborts_before_writing_when_settings_cannot_be_read(
        refresh: RefreshPolicy,
    ) {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_reads(Key::Settings, vec![Err(WIN32_ERROR(5))]);
        let error = nightlight
            .set_color_temperature("3400".parse().expect("valid temperature"), refresh)
            .expect_err("settings read is denied");
        assert_eq!(error.kind(), ErrorKind::Registry);
        assert_eq!(store.reads(), [Key::Settings]);
        assert_eq!(store.writes(), []);
    }

    #[test_case(false; "cleanup writes fail")]
    #[test_case(true; "cleanup reads fail")]
    fn always_refresh_recovers_after_exhausted_cleanup(read_failure: bool) {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_PREVIEWING)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        if read_failure {
            store.queue_reads(
                Key::Settings,
                vec![
                    Ok(fixtures::SETTINGS_PREVIEWING.to_vec()),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                ],
            );
        } else {
            store.queue_writes(
                Key::Settings,
                vec![
                    Ok(()),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                ],
            );
        }
        let temperature = "2790".parse().expect("valid temperature");
        let error = nightlight
            .set_color_temperature(temperature, RefreshPolicy::Always)
            .expect_err("cleanup fails after temperature storage");
        assert_eq!(error.kind(), ErrorKind::PreviewCleanup);
        assert!(stored_settings(&store).is_previewing());
        assert_eq!(stored_settings(&store).color_temperature(), Some(2790));
        assert!(
            nightlight
                .set_color_temperature(temperature, RefreshPolicy::Always)
                .expect("next operation repeats the full refresh")
        );
        let first_attempt_writes = if read_failure { 1 } else { 4 };
        assert_eq!(store.writes().len(), first_attempt_writes + 2);
        for ((_, bytes), (previewing, seconds)) in store
            .write_attempts()
            .iter()
            .skip(first_attempt_writes)
            .zip([(true, 1_800_000_002), (false, 1_800_000_004)])
        {
            let settings = Settings::from_bytes(bytes).expect("written settings decode");
            assert_eq!(settings.is_previewing(), previewing);
            assert_eq!(settings.color_temperature(), Some(2790));
            assert_eq!(
                settings.modified(),
                Some(UNIX_EPOCH + Duration::from_secs(seconds))
            );
        }
        assert!(!stored_settings(&store).is_previewing());
    }

    #[test_case("3400", true; "changed temperature")]
    #[test_case("2790", false; "same temperature")]
    fn preserves_existing_preview_without_reading_state(temperature: &str, changed: bool) {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_PREVIEWING)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        assert_eq!(
            nightlight
                .set_color_temperature(
                    temperature.parse().expect("valid temperature"),
                    RefreshPolicy::Automatic
                )
                .expect("settings update"),
            changed
        );
        assert!(stored_settings(&store).is_previewing());
        assert_eq!(store.reads(), [Key::Settings]);
        assert_eq!(store.writes().len(), usize::from(changed));
    }

    #[test_case(RefreshPolicy::Automatic; "automatic")]
    #[test_case(RefreshPolicy::Always; "always")]
    fn unchanged_active_temperature_matches_native_preview_sequence_with_future_timestamps(
        refresh: RefreshPolicy,
    ) {
        let initial = hex!(
            "434201000A0201002A06BCE6C5D6062A2B0E2143420100CA140E1500CA1E0E0700CF28B231CA320E132E1100CA3C0E062E260000000000"
        );
        let store = MemoryStore::with(&[
            (Key::Settings, &initial),
            (Key::State, fixtures::STATE_ACTIVE),
        ]);
        let nightlight = Nightlight::with_store(store.clone(), clock_before_fixtures);
        assert!(
            nightlight
                .set_color_temperature("3161".parse().expect("valid temperature"), refresh)
                .expect("preview refreshes")
        );
        assert_eq!(store.write_attempts(), vec![
            (Key::Settings, hex!("434201000A0201002A06BEE6C5D6062A2B0E2443420100CA140E1500CA1E0E0700CF28B231CA320E132E1100CA3C0E062E2600C2460100000000").to_vec()),
            (Key::Settings, hex!("434201000A0201002A06C0E6C5D6062A2B0E2143420100CA140E1500CA1E0E0700CF28B231CA320E132E1100CA3C0E062E260000000000").to_vec()),
        ]);
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_ACTIVE)
        );
    }

    #[test_case(RefreshPolicy::Automatic; "automatic")]
    #[test_case(RefreshPolicy::Always; "always")]
    fn preview_cleanup_preserves_newer_temperature_schedule_unknown_fields_and_timestamp(
        refresh: RefreshPolicy,
    ) {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        let newer = hex!(
            "43420100 0A020100 2A 068AA4A7DA06 2A 0605 2B0E2A"
            "43420100 C20A00 CA140E012E0F00 CA1E00 CF28C03E D02D0A CA320E132E1700 CA3C0E072E0C00 C24601 00"
            "00 4201 00 4607 00"
        );
        store.queue_reads(
            Key::Settings,
            vec![
                Ok(fixtures::SETTINGS_SET_HOURS.to_vec()),
                Ok(newer.to_vec()),
            ],
        );
        nightlight
            .set_color_temperature("3400".parse().expect("valid temperature"), refresh)
            .expect("temperature updates");
        assert_eq!(
            store.value(Key::Settings).expect("settings exist"),
            hex!(
                "43420100 0A020100 2A 068CA4A7DA06 2A 0605 2B0E27"
                "43420100 C20A00 CA140E012E0F00 CA1E00 CF28C03E D02D0A CA320E132E1700 CA3C0E072E0C00 00"
                "00 4201 00 4607 00"
            )
        );
        assert_eq!(store.writes(), [Key::Settings, Key::Settings]);
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_ACTIVE)
        );
    }

    #[test]
    fn temperature_update_aborts_before_writing_when_state_cannot_be_read() {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_SET_HOURS)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        let error = nightlight
            .set_color_temperature(
                "3400".parse().expect("valid temperature"),
                RefreshPolicy::Automatic,
            )
            .expect_err("state is missing");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(store.writes(), []);
    }

    #[test_case(RefreshPolicy::Automatic; "automatic")]
    #[test_case(RefreshPolicy::Always; "always")]
    fn failed_preview_activation_does_not_run_cleanup(refresh: RefreshPolicy) {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_writes(Key::Settings, vec![Err(WIN32_ERROR(5))]);
        let error = nightlight
            .set_color_temperature("3400".parse().expect("valid temperature"), refresh)
            .expect_err("write is denied");
        assert_eq!(error.kind(), ErrorKind::Registry);
        assert_eq!(store.writes(), [Key::Settings]);
        assert_eq!(
            store.value(Key::Settings).as_deref(),
            Some(fixtures::SETTINGS_SET_HOURS)
        );
    }

    #[test_case(RefreshPolicy::Automatic; "automatic")]
    #[test_case(RefreshPolicy::Always; "always")]
    fn retries_preview_cleanup_after_a_failed_write(refresh: RefreshPolicy) {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_writes(Key::Settings, vec![Ok(()), Err(WIN32_ERROR(5)), Ok(())]);
        assert!(
            nightlight
                .set_color_temperature("3400".parse().expect("valid temperature"), refresh)
                .expect("cleanup retry succeeds")
        );
        assert_eq!(
            store.writes(),
            [Key::Settings, Key::Settings, Key::Settings]
        );
        assert!(!stored_settings(&store).is_previewing());
        assert_eq!(stored_settings(&store).color_temperature(), Some(3400));
        assert_eq!(
            stored_settings(&store).modified(),
            Some(UNIX_EPOCH + Duration::from_secs(1_800_000_002))
        );
    }

    #[test_case(RefreshPolicy::Automatic; "automatic")]
    #[test_case(RefreshPolicy::Always; "always")]
    fn retries_preview_cleanup_after_a_failed_read(refresh: RefreshPolicy) {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_reads(
            Key::Settings,
            vec![
                Ok(fixtures::SETTINGS_SET_HOURS.to_vec()),
                Err(WIN32_ERROR(5)),
            ],
        );
        assert!(
            nightlight
                .set_color_temperature("3400".parse().expect("valid temperature"), refresh)
                .expect("cleanup retry succeeds")
        );
        assert_eq!(store.writes(), [Key::Settings, Key::Settings]);
        assert_eq!(
            store.reads(),
            match refresh {
                RefreshPolicy::Automatic =>
                    vec![Key::Settings, Key::State, Key::Settings, Key::Settings],
                RefreshPolicy::Always => vec![Key::Settings, Key::Settings, Key::Settings],
            }
        );
        assert!(!stored_settings(&store).is_previewing());
        assert_eq!(stored_settings(&store).color_temperature(), Some(3400));
    }

    #[test_case(false, RefreshPolicy::Automatic; "automatic cleanup writes fail")]
    #[test_case(true, RefreshPolicy::Automatic; "automatic cleanup reads fail")]
    #[test_case(false, RefreshPolicy::Always; "always cleanup writes fail")]
    #[test_case(true, RefreshPolicy::Always; "always cleanup reads fail")]
    fn reports_exhausted_preview_cleanup_with_the_underlying_error(
        read_failure: bool,
        refresh: RefreshPolicy,
    ) {
        use std::error::Error as _;

        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        if read_failure {
            store.queue_reads(
                Key::Settings,
                vec![
                    Ok(fixtures::SETTINGS_SET_HOURS.to_vec()),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                ],
            );
        } else {
            store.queue_writes(
                Key::Settings,
                vec![
                    Ok(()),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                    Err(WIN32_ERROR(5)),
                ],
            );
        }
        let error = nightlight
            .set_color_temperature("3400".parse().expect("valid temperature"), refresh)
            .expect_err("cleanup cannot finish");
        assert_eq!(error.kind(), ErrorKind::PreviewCleanup);
        assert_eq!(
            error.to_string(),
            "clearing Night Light preview after writing the temperature; preview may remain active"
        );
        let source = error.source().expect("cleanup error has a source");
        assert_eq!(
            source.to_string(),
            if read_failure {
                "reading Night Light settings from the registry"
            } else {
                "writing Night Light settings to the registry"
            }
        );
        let native = source
            .source()
            .expect("registry error has a source")
            .downcast_ref::<windows_result::Error>()
            .expect("source is a Windows error");
        assert_eq!(native.code(), WIN32_ERROR(5).to_hresult());
        assert_eq!(store.writes().len(), if read_failure { 1 } else { 4 });
        assert!(stored_settings(&store).is_previewing());
        assert_eq!(stored_settings(&store).color_temperature(), Some(3400));
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_ACTIVE)
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
