use crate::ColorTemperature;
use crate::Result;
use crate::Settings;
use crate::State;
use crate::error::Repr;
use crate::store::Key;
use crate::store::RegistryStore;
use crate::store::Store;
use std::fmt;
use std::time::Duration;
use std::time::SystemTime;

const READ_ATTEMPTS: usize = 3;
const CLEANUP_ATTEMPTS: usize = 3;
// After an `explorer.exe` restart, Windows ignores settings writes until a
// preview stays on for about 9 ms or longer. A click on the Windows Settings
// slider keeps preview on for 51 to 97 ms.
const APPLY_PREVIEW_HOLD: Duration = Duration::from_millis(50);

/// Handle to the current user's Night Light settings and state.
///
/// Settings (schedule and color temperature) and state (whether Night Light
/// is on) are separate registry values. Temperature updates can read both
/// values and write only settings; on/off updates write only state.
pub struct Nightlight {
    store: Box<dyn Store + Send + Sync>,
    clock: fn() -> SystemTime,
    sleep: fn(Duration),
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
            sleep: std::thread::sleep,
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

    /// Store `temperature` in one write, without preview.
    ///
    /// While Night Light is on, Windows applies the stored temperature, except
    /// after an `explorer.exe` restart, when it ignores such writes until a
    /// preview stays on for about 9 ms or longer.
    /// [`apply_color_temperature`](Self::apply_color_temperature) makes such a
    /// preview.
    ///
    /// - Return whether it wrote the settings; it does not write when the
    ///   temperature is already stored.
    /// - Each write advances the settings timestamp by at least two seconds.
    /// - Do not sleep, do not change preview, and do not read or write the
    ///   on/off state.
    ///
    /// # Errors
    ///
    /// Returns any error of [`update_settings`](Self::update_settings).
    pub fn set_color_temperature(&self, temperature: ColorTemperature) -> Result<bool> {
        self.update_settings(|settings| settings.set_color_temperature(temperature))
    }

    /// Show `temperature` now while Night Light is on, or store it for when
    /// Night Light turns on, and end any preview.
    ///
    /// - Read the settings and the on/off state before writing.
    /// - While Night Light is on, write `temperature` with preview on, wait 50
    ///   ms, then turn preview off. Windows applies a preview this long even
    ///   after an `explorer.exe` restart; a click on the Night Light slider in
    ///   Windows Settings keeps preview on for a similar 51 to 97 ms. The call
    ///   blocks the thread for the wait, so run it on a blocking thread in
    ///   async code.
    /// - While Night Light is off, store `temperature` with preview off in one
    ///   write, so the screen stays untinted. Do not write when both are
    ///   already stored.
    /// - Return whether preview was on before the call, for example during a
    ///   fade or a slider drag in Windows Settings; this call ends it.
    /// - Each write advances the settings timestamp by at least two seconds.
    ///
    /// # Errors
    ///
    /// - Return any error of [`read_settings`](Self::read_settings),
    ///   [`read_state`](Self::read_state), or
    ///   [`write_settings`](Self::write_settings).
    /// - Return [`ErrorKind::PreviewCleanup`] if turning preview off after the
    ///   wait fails three times in a row, without waiting between attempts.
    ///   Preview may stay on, and while it does, Windows shows the stored
    ///   temperature even while Night Light is off. The error's source is the
    ///   last failure. Calling this method again repeats the whole sequence.
    ///
    /// [`ErrorKind::PreviewCleanup`]: crate::ErrorKind::PreviewCleanup
    pub fn apply_color_temperature(&self, temperature: ColorTemperature) -> Result<bool> {
        let current = self.read_settings()?;
        let active = self.read_state()?.is_active();
        let mut settings = current.clone();
        settings.set_color_temperature(temperature);
        settings.set_previewing(active);
        if active {
            self.write_settings(&settings)?;
            (self.sleep)(APPLY_PREVIEW_HOLD);
            self.clear_preview()?;
        } else if settings != current {
            self.write_settings(&settings)?;
        }
        Ok(current.is_previewing())
    }

    fn clear_preview(&self) -> Result<()> {
        let mut attempt = 1;
        loop {
            match self.update_settings(|settings| settings.set_previewing(false)) {
                Ok(_) => return Ok(()),
                Err(source) if attempt == CLEANUP_ATTEMPTS => {
                    return Err(Repr::PreviewCleanup {
                        source: Box::new(source),
                    }
                    .into());
                }
                Err(_) => attempt += 1,
            }
        }
    }

    /// Turn preview on without changing the temperature, as pressing the
    /// Night Light slider in Windows Settings does.
    ///
    /// While preview is on, Windows applies the stored temperature, even while
    /// Night Light is off. Change it with
    /// [`preview_color_temperature`](Self::preview_color_temperature) and turn
    /// preview off with [`end_preview`](Self::end_preview).
    ///
    /// - Return whether preview was already on.
    /// - Write even when preview is already on. Each write advances the
    ///   settings timestamp by at least two seconds.
    /// - Do not sleep, and do not read or write the on/off state.
    ///
    /// # Errors
    ///
    /// Returns any error of [`read_settings`](Self::read_settings) or
    /// [`write_settings`](Self::write_settings).
    pub fn start_preview(&self) -> Result<bool> {
        self.write_preview(|settings| settings.set_previewing(true))
    }

    /// Write `temperature` with preview on, as dragging the Night Light slider
    /// in Windows Settings does.
    ///
    /// - Return whether preview was already on. `false` means that preview was
    ///   off before this call, for example because another writer ended it;
    ///   this call turns it back on.
    /// - Write even when the temperature and preview already match. Each write
    ///   advances the settings timestamp by at least two seconds.
    /// - Do not sleep, and do not read or write the on/off state.
    ///
    /// # Errors
    ///
    /// Returns any error of [`read_settings`](Self::read_settings) or
    /// [`write_settings`](Self::write_settings).
    pub fn preview_color_temperature(&self, temperature: ColorTemperature) -> Result<bool> {
        self.write_preview(|settings| {
            settings.set_color_temperature(temperature);
            settings.set_previewing(true);
        })
    }

    /// Turn preview off and keep the temperature, as releasing the Night
    /// Light slider in Windows Settings does.
    ///
    /// - Return whether preview was on.
    /// - Write even when preview is already off, in one attempt. Each write
    ///   advances the settings timestamp by at least two seconds.
    /// - Do not sleep, and do not read or write the on/off state.
    ///
    /// # Errors
    ///
    /// Returns any error of [`read_settings`](Self::read_settings) or
    /// [`write_settings`](Self::write_settings). A failed write does not
    /// change the stored settings, so preview stays on if it was on.
    pub fn end_preview(&self) -> Result<bool> {
        self.write_preview(|settings| settings.set_previewing(false))
    }

    fn write_preview(&self, change: impl FnOnce(&mut Settings)) -> Result<bool> {
        let mut settings = self.read_settings()?;
        let was_previewing = settings.is_previewing();
        change(&mut settings);
        self.write_settings(&settings)?;
        Ok(was_previewing)
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
    use std::cell::RefCell;
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

    thread_local! {
        static SLEEPS: RefCell<Vec<Duration>> = const { RefCell::new(Vec::new()) };
    }

    fn record_sleep(duration: Duration) {
        SLEEPS.with_borrow_mut(|sleeps| sleeps.push(duration));
    }

    fn sleeps() -> Vec<Duration> {
        SLEEPS.with_borrow(Clone::clone)
    }

    fn applying_with_store(store: &MemoryStore, clock: fn() -> SystemTime) -> Nightlight {
        // libtest runs a test on the current thread when it cannot spawn one,
        // so an earlier test's sleeps can remain.
        SLEEPS.with_borrow_mut(Vec::clear);
        Nightlight {
            sleep: record_sleep,
            ..Nightlight::with_store(store.clone(), clock)
        }
    }

    fn written_settings(store: &MemoryStore) -> Vec<(bool, Option<i16>, Option<SystemTime>)> {
        store
            .write_attempts()
            .iter()
            .map(|(key, bytes)| {
                assert_eq!(*key, Key::Settings);
                let settings = Settings::from_bytes(bytes).expect("written settings decode");
                (
                    settings.is_previewing(),
                    settings.color_temperature(),
                    settings.modified(),
                )
            })
            .collect()
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn set_color_temperature_writes_only_settings_without_preview() {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        let temperature = ColorTemperature::new(3400).expect("3400K is in range");
        assert!(
            nightlight
                .set_color_temperature(temperature)
                .expect("temperature is stored")
        );
        assert_eq!(store.write_attempts(), vec![(
            Key::Settings,
            hex!(
                "43420100 0A020100 2A 0680A4A7DA06 2A 2B0E26"
                "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF289035 CA320E132E1700 CA3C0E072E0C00 00"
                "00 00 00"
            )
            .to_vec()
        )]);
        assert_eq!(store.reads(), [Key::Settings]);
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_ACTIVE)
        );
    }

    #[test]
    fn set_color_temperature_does_not_write_unchanged_temperature() {
        let (nightlight, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        let temperature = ColorTemperature::new(2790).expect("2790K is in range");
        assert!(
            !nightlight
                .set_color_temperature(temperature)
                .expect("settings read")
        );
        assert_eq!(store.writes(), []);
    }

    #[test]
    fn set_color_temperature_keeps_preview_without_reading_state() {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_PREVIEWING)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        let temperature = ColorTemperature::new(3400).expect("3400K is in range");
        assert!(
            nightlight
                .set_color_temperature(temperature)
                .expect("temperature is stored")
        );
        let settings = stored_settings(&store);
        assert!(settings.is_previewing());
        assert_eq!(settings.color_temperature(), Some(3400));
        assert_eq!(store.reads(), [Key::Settings]);
    }

    #[test_case(fixtures::STATE_ACTIVE; "manual activation")]
    #[test_case(fixtures::STATE_ACTIVE_SCHEDULED; "scheduled activation")]
    fn apply_while_active_writes_preview_then_ends_it(state: &[u8]) {
        let store = MemoryStore::with(&[
            (Key::Settings, fixtures::SETTINGS_SET_HOURS),
            (Key::State, state),
        ]);
        let nightlight = applying_with_store(&store, clock);
        let temperature = ColorTemperature::new(3400).expect("3400K is in range");
        assert!(
            !nightlight
                .apply_color_temperature(temperature)
                .expect("temperature applies")
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
        assert_eq!(sleeps(), [Duration::from_millis(50)]);
        assert_eq!(store.reads(), [Key::Settings, Key::State, Key::Settings]);
        assert_eq!(store.value(Key::State).as_deref(), Some(state));
    }

    #[test_case(fixtures::SETTINGS_SET_HOURS, 2790, false; "unchanged temperature")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, 2790, true; "preview on with unchanged temperature")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, 3400, true; "preview on with changed temperature")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, 1200, true; "minimum temperature")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, 6500, true; "maximum temperature")]
    fn apply_while_active_always_writes_and_ends_preview(
        initial: &[u8],
        kelvin: u16,
        was_previewing: bool,
    ) {
        let store = MemoryStore::with(&[
            (Key::Settings, initial),
            (Key::State, fixtures::STATE_ACTIVE),
        ]);
        let nightlight = applying_with_store(&store, clock);
        let temperature = ColorTemperature::new(kelvin).expect("temperature is in range");
        assert_eq!(
            nightlight
                .apply_color_temperature(temperature)
                .expect("temperature applies"),
            was_previewing
        );
        let kelvin = Some(kelvin.cast_signed());
        assert_eq!(
            written_settings(&store),
            [
                (true, kelvin, Some(at(1_800_000_000))),
                (false, kelvin, Some(at(1_800_000_002))),
            ]
        );
        assert_eq!(stored_settings(&store).mode(), ScheduleMode::SetHours);
        assert_eq!(sleeps(), [Duration::from_millis(50)]);
    }

    #[test_case(fixtures::SETTINGS_SET_HOURS, 3400, false, 1; "changed temperature")]
    #[test_case(fixtures::SETTINGS_SET_HOURS, 2790, false, 0; "unchanged temperature")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, 3400, true, 1; "preview on with changed temperature")]
    #[test_case(fixtures::SETTINGS_PREVIEWING, 2790, true, 1; "preview on with unchanged temperature")]
    #[test_case(fixtures::SETTINGS_LIVE, 3400, false, 1; "schedule off")]
    fn apply_while_inactive_stores_temperature_with_preview_off(
        initial: &[u8],
        kelvin: u16,
        was_previewing: bool,
        writes: usize,
    ) {
        let store = MemoryStore::with(&[
            (Key::Settings, initial),
            (Key::State, fixtures::STATE_INACTIVE),
        ]);
        let nightlight = applying_with_store(&store, clock);
        let temperature = ColorTemperature::new(kelvin).expect("temperature is in range");
        assert_eq!(
            nightlight
                .apply_color_temperature(temperature)
                .expect("temperature is stored"),
            was_previewing
        );
        assert_eq!(store.writes().len(), writes);
        let settings = stored_settings(&store);
        assert!(!settings.is_previewing());
        assert_eq!(settings.color_temperature(), Some(kelvin.cast_signed()));
        assert_eq!(
            settings.mode(),
            Settings::from_bytes(initial)
                .expect("initial settings decode")
                .mode()
        );
        assert_eq!(sleeps(), []);
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_INACTIVE)
        );
    }

    #[test]
    fn apply_does_not_write_when_state_cannot_be_read() {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_SET_HOURS)]);
        let nightlight = applying_with_store(&store, clock);
        let error = nightlight
            .apply_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"))
            .expect_err("state is missing");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(store.writes(), []);
    }

    type TemperatureOperation = fn(&Nightlight, ColorTemperature) -> Result<bool>;

    #[test_case(Nightlight::set_color_temperature; "set")]
    #[test_case(Nightlight::apply_color_temperature; "apply")]
    fn temperature_operation_does_not_write_when_settings_cannot_be_read(
        operation: TemperatureOperation,
    ) {
        let store = MemoryStore::with(&[
            (Key::Settings, fixtures::SETTINGS_SET_HOURS),
            (Key::State, fixtures::STATE_ACTIVE),
        ]);
        store.queue_reads(Key::Settings, vec![Err(WIN32_ERROR(5))]);
        let nightlight = applying_with_store(&store, clock);
        let temperature = ColorTemperature::new(3400).expect("3400K is in range");
        let error = operation(&nightlight, temperature).expect_err("settings read is denied");
        assert_eq!(error.kind(), ErrorKind::Registry);
        assert_eq!(store.reads(), [Key::Settings]);
        assert_eq!(store.writes(), []);
    }

    #[test]
    fn failed_apply_write_does_not_wait_or_end_preview() {
        let (_, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_writes(Key::Settings, vec![Err(WIN32_ERROR(5))]);
        let nightlight = applying_with_store(&store, clock);
        let error = nightlight
            .apply_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"))
            .expect_err("write is denied");
        assert_eq!(error.kind(), ErrorKind::Registry);
        assert_eq!(store.writes(), [Key::Settings]);
        assert_eq!(sleeps(), []);
        assert_eq!(
            store.value(Key::Settings).as_deref(),
            Some(fixtures::SETTINGS_SET_HOURS)
        );
    }

    #[test]
    fn apply_retries_ending_preview_after_a_failed_write() {
        let (_, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_writes(Key::Settings, vec![Ok(()), Err(WIN32_ERROR(5)), Ok(())]);
        let nightlight = applying_with_store(&store, clock);
        nightlight
            .apply_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"))
            .expect("cleanup retry succeeds");
        assert_eq!(
            store.writes(),
            [Key::Settings, Key::Settings, Key::Settings]
        );
        let settings = stored_settings(&store);
        assert!(!settings.is_previewing());
        assert_eq!(settings.color_temperature(), Some(3400));
        assert_eq!(settings.modified(), Some(at(1_800_000_002)));
    }

    #[test]
    fn apply_retries_ending_preview_after_a_failed_read() {
        let (_, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        store.queue_reads(
            Key::Settings,
            vec![
                Ok(fixtures::SETTINGS_SET_HOURS.to_vec()),
                Err(WIN32_ERROR(5)),
            ],
        );
        let nightlight = applying_with_store(&store, clock);
        nightlight
            .apply_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"))
            .expect("cleanup retry succeeds");
        assert_eq!(store.writes(), [Key::Settings, Key::Settings]);
        assert_eq!(
            store.reads(),
            [Key::Settings, Key::State, Key::Settings, Key::Settings]
        );
        let settings = stored_settings(&store);
        assert!(!settings.is_previewing());
        assert_eq!(settings.color_temperature(), Some(3400));
    }

    fn fail_cleanup(store: &MemoryStore, initial: &[u8], read_failure: bool) {
        let denied = WIN32_ERROR(5);
        if read_failure {
            store.queue_reads(
                Key::Settings,
                vec![Ok(initial.to_vec()), Err(denied), Err(denied), Err(denied)],
            );
        } else {
            store.queue_writes(
                Key::Settings,
                vec![Ok(()), Err(denied), Err(denied), Err(denied)],
            );
        }
    }

    #[test_case(false; "cleanup writes fail")]
    #[test_case(true; "cleanup reads fail")]
    fn apply_reports_exhausted_preview_cleanup_with_the_underlying_error(read_failure: bool) {
        use std::error::Error as _;

        let (_, store) = nightlight(fixtures::STATE_ACTIVE, clock);
        fail_cleanup(&store, fixtures::SETTINGS_SET_HOURS, read_failure);
        let nightlight = applying_with_store(&store, clock);
        let error = nightlight
            .apply_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"))
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
        let settings = stored_settings(&store);
        assert!(settings.is_previewing());
        assert_eq!(settings.color_temperature(), Some(3400));
        assert_eq!(
            store.value(Key::State).as_deref(),
            Some(fixtures::STATE_ACTIVE)
        );
    }

    #[test_case(false; "cleanup writes fail")]
    #[test_case(true; "cleanup reads fail")]
    fn apply_repeats_the_whole_sequence_after_exhausted_cleanup(read_failure: bool) {
        let store = MemoryStore::with(&[
            (Key::Settings, fixtures::SETTINGS_PREVIEWING),
            (Key::State, fixtures::STATE_ACTIVE),
        ]);
        fail_cleanup(&store, fixtures::SETTINGS_PREVIEWING, read_failure);
        let nightlight = applying_with_store(&store, clock);
        let temperature = ColorTemperature::new(2790).expect("2790K is in range");
        let error = nightlight
            .apply_color_temperature(temperature)
            .expect_err("cleanup fails after the preview write");
        assert_eq!(error.kind(), ErrorKind::PreviewCleanup);
        assert!(stored_settings(&store).is_previewing());
        assert!(
            nightlight
                .apply_color_temperature(temperature)
                .expect("the next call repeats the whole sequence")
        );
        let first_call_writes = if read_failure { 1 } else { 4 };
        assert_eq!(
            written_settings(&store)[first_call_writes..],
            [
                (true, Some(2790), Some(at(1_800_000_002))),
                (false, Some(2790), Some(at(1_800_000_004))),
            ]
        );
    }

    #[test_case(false; "fixed clock")]
    #[test_case(true; "clock moves backward")]
    fn repeated_apply_advances_timestamps_with_unchanged_temperature(clock_moves_backward: bool) {
        let store = MemoryStore::with(&[
            (Key::Settings, fixtures::SETTINGS_PREVIEWING),
            (Key::State, fixtures::STATE_ACTIVE),
        ]);
        let temperature = ColorTemperature::new(2790).expect("2790K is in range");
        applying_with_store(&store, clock)
            .apply_color_temperature(temperature)
            .expect("first call applies");
        let next_clock = if clock_moves_backward {
            clock_before_fixtures
        } else {
            clock
        };
        applying_with_store(&store, next_clock)
            .apply_color_temperature(temperature)
            .expect("second call applies");
        assert_eq!(
            written_settings(&store),
            [
                (true, Some(2790), Some(at(1_800_000_000))),
                (false, Some(2790), Some(at(1_800_000_002))),
                (true, Some(2790), Some(at(1_800_000_004))),
                (false, Some(2790), Some(at(1_800_000_006))),
            ]
        );
    }

    #[test]
    fn apply_advances_a_future_timestamp_by_two_seconds_per_write() {
        let initial = hex!(
            "434201000A0201002A06BCE6C5D6062A2B0E2143420100CA140E1500CA1E0E0700CF28B231CA320E132E1100CA3C0E062E260000000000"
        );
        let store = MemoryStore::with(&[
            (Key::Settings, &initial),
            (Key::State, fixtures::STATE_ACTIVE),
        ]);
        let nightlight = applying_with_store(&store, clock_before_fixtures);
        nightlight
            .apply_color_temperature(ColorTemperature::new(3161).expect("3161K is in range"))
            .expect("temperature applies");
        assert_eq!(store.write_attempts(), vec![
            (Key::Settings, hex!("434201000A0201002A06BEE6C5D6062A2B0E2443420100CA140E1500CA1E0E0700CF28B231CA320E132E1100CA3C0E062E2600C2460100000000").to_vec()),
            (Key::Settings, hex!("434201000A0201002A06C0E6C5D6062A2B0E2143420100CA140E1500CA1E0E0700CF28B231CA320E132E1100CA3C0E062E260000000000").to_vec()),
        ]);
    }

    #[test]
    fn apply_cleanup_preserves_newer_temperature_schedule_unknown_fields_and_timestamp() {
        let (_, store) = nightlight(fixtures::STATE_ACTIVE, clock);
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
        let nightlight = applying_with_store(&store, clock);
        nightlight
            .apply_color_temperature(ColorTemperature::new(3400).expect("3400K is in range"))
            .expect("temperature applies");
        assert_eq!(
            store.value(Key::Settings).expect("settings exist"),
            hex!(
                "43420100 0A020100 2A 068CA4A7DA06 2A 0605 2B0E27"
                "43420100 C20A00 CA140E012E0F00 CA1E00 CF28C03E D02D0A CA320E132E1700 CA3C0E072E0C00 00"
                "00 4201 00 4607 00"
            )
        );
        assert_eq!(store.writes(), [Key::Settings, Key::Settings]);
    }

    type PreviewOperation = fn(&Nightlight) -> Result<bool>;

    fn start(nightlight: &Nightlight) -> Result<bool> {
        nightlight.start_preview()
    }

    fn step(nightlight: &Nightlight) -> Result<bool> {
        nightlight.preview_color_temperature(ColorTemperature::new(3400)?)
    }

    fn end(nightlight: &Nightlight) -> Result<bool> {
        nightlight.end_preview()
    }

    #[test]
    fn start_preview_writes_preview_with_unchanged_temperature() {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_SET_HOURS)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        assert!(!nightlight.start_preview().expect("preview starts"));
        assert_eq!(store.write_attempts(), vec![(
            Key::Settings,
            hex!(
                "43420100 0A020100 2A 0680A4A7DA06 2A 2B0E29"
                "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF28CC2B CA320E132E1700 CA3C0E072E0C00 C24601 00"
                "00 00 00"
            )
            .to_vec()
        )]);
        assert_eq!(store.reads(), [Key::Settings]);
    }

    #[test]
    fn drag_writes_each_step_with_advancing_timestamps() {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_SET_HOURS)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        let warm = ColorTemperature::new(3400).expect("3400K is in range");
        let warmer = ColorTemperature::new(2400).expect("2400K is in range");
        let was_previewing = [
            nightlight.start_preview().expect("preview starts"),
            nightlight
                .preview_color_temperature(warm)
                .expect("first step writes"),
            nightlight
                .preview_color_temperature(warmer)
                .expect("second step writes"),
            nightlight
                .preview_color_temperature(warmer)
                .expect("repeated step writes"),
            nightlight.end_preview().expect("preview ends"),
        ];
        assert_eq!(was_previewing, [false, true, true, true, true]);
        assert_eq!(store.reads(), [Key::Settings; 5]);
        assert_eq!(
            written_settings(&store),
            [
                (true, Some(2790), Some(at(1_800_000_000))),
                (true, Some(3400), Some(at(1_800_000_002))),
                (true, Some(2400), Some(at(1_800_000_004))),
                (true, Some(2400), Some(at(1_800_000_006))),
                (false, Some(2400), Some(at(1_800_000_008))),
            ]
        );
    }

    #[test_case(start, fixtures::SETTINGS_SET_HOURS, false; "start from off")]
    #[test_case(start, fixtures::SETTINGS_PREVIEWING, true; "start while on")]
    #[test_case(step, fixtures::SETTINGS_SET_HOURS, false; "step from off")]
    #[test_case(step, fixtures::SETTINGS_PREVIEWING, true; "step while on")]
    #[test_case(end, fixtures::SETTINGS_SET_HOURS, false; "end from off")]
    #[test_case(end, fixtures::SETTINGS_PREVIEWING, true; "end while on")]
    fn preview_operation_reports_prior_preview_and_always_writes(
        operation: PreviewOperation,
        initial: &[u8],
        was_previewing: bool,
    ) {
        let store = MemoryStore::with(&[(Key::Settings, initial)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        assert_eq!(
            operation(&nightlight).expect("preview write succeeds"),
            was_previewing
        );
        assert_eq!(store.reads(), [Key::Settings]);
        assert_eq!(store.writes(), [Key::Settings]);
    }

    #[test_case(start, true, 2790; "start")]
    #[test_case(step, true, 3400; "step")]
    #[test_case(end, false, 2790; "end")]
    fn preview_operation_writes_preview_and_temperature(
        operation: PreviewOperation,
        previewing: bool,
        kelvin: i16,
    ) {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_PREVIEWING)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        operation(&nightlight).expect("preview write succeeds");
        let settings = stored_settings(&store);
        assert_eq!(settings.is_previewing(), previewing);
        assert_eq!(settings.color_temperature(), Some(kelvin));
        assert_eq!(settings.mode(), ScheduleMode::SetHours);
    }

    #[test]
    fn preview_step_preserves_unknown_fields() {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_WITH_UNKNOWN_FIELDS)]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        assert!(!step(&nightlight).expect("step writes"));
        assert_eq!(
            store.value(Key::Settings).expect("settings exist"),
            hex!(
                "43420100 0A020100 2A 0680A4A7DA06 2A 0605 2B0E2C"
                "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF289035 D02D0A CA320E132E1700 CA3C0E072E0C00 C24601 00"
                "00 4201 00 4607 00"
            )
        );
    }

    #[test_case(start; "start")]
    #[test_case(step; "step")]
    #[test_case(end; "end")]
    fn preview_operation_does_not_write_when_settings_cannot_be_read(operation: PreviewOperation) {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_PREVIEWING)]);
        store.queue_reads(Key::Settings, vec![Err(WIN32_ERROR(5))]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        let error = operation(&nightlight).expect_err("settings read is denied");
        assert_eq!(error.kind(), ErrorKind::Registry);
        assert_eq!(store.writes(), []);
    }

    #[test_case(start; "start")]
    #[test_case(step; "step")]
    #[test_case(end; "end")]
    fn preview_operation_makes_one_write_attempt(operation: PreviewOperation) {
        let store = MemoryStore::with(&[(Key::Settings, fixtures::SETTINGS_PREVIEWING)]);
        store.queue_writes(Key::Settings, vec![Err(WIN32_ERROR(5))]);
        let nightlight = Nightlight::with_store(store.clone(), clock);
        let error = operation(&nightlight).expect_err("settings write is denied");
        assert_eq!(error.kind(), ErrorKind::Registry);
        assert_eq!(store.writes(), [Key::Settings]);
        assert_eq!(
            store.value(Key::Settings).as_deref(),
            Some(fixtures::SETTINGS_PREVIEWING)
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
