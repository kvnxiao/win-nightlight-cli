//! Raw access to the `Current` settings value and its `Cloud` record.

use crate::envelope;
use crate::envelope::SettingsEnvelope;
use crate::snapshot::unix_secs;
use anyhow::Context as _;
use std::time::Duration;
use std::time::SystemTime;
use win_nightlight_lib::ColorTemperature;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::SETTINGS_KEY_PATH;
use windows_registry::CURRENT_USER;
use windows_registry::Type;

const VALUE_NAME: &str = "Data";
const MODIFIED_STEP_SECS: u64 = 2;
// After an explorer.exe restart, explorer.exe ignored every preview whose on
// and off values it read 8 ms apart or less, and applied every preview with
// 50 ms steps.
pub(crate) const PREVIEW_STEP: Duration = Duration::from_millis(50);

fn cloud_key_path() -> String {
    SETTINGS_KEY_PATH.replacen(r"\Current\", r"\Cloud\", 1)
}

/// Read the timestamp of the latest Windows Settings write.
pub(crate) fn read_cloud_modified() -> anyhow::Result<u64> {
    let data = CURRENT_USER
        .options()
        .read()
        .open(cloud_key_path())
        .and_then(|key| key.get_bytes(VALUE_NAME))
        .context(
            "reading the Cloud settings record; move the Night Light Strength slider in \
             Windows Settings once to create it",
        )?;
    envelope::cloud_modified(&data)
}

pub(crate) fn write_cloud_modified(modified: u64) -> anyhow::Result<()> {
    CURRENT_USER
        .options()
        .write()
        .open(cloud_key_path())
        .and_then(|key| key.set_bytes(VALUE_NAME, Type::Bytes, &envelope::cloud_record(modified)))
        .context("writing the Cloud settings record")
}

/// Store `modified` as the settings timestamp, even when it is older than the
/// stored one, without changing any setting.
pub(crate) fn write_settings_modified(modified: u64) -> anyhow::Result<()> {
    patch_settings(|envelope| {
        envelope.modified = modified;
        Ok(())
    })
}

/// Set or clear the preview flag, stamping the write two seconds after the
/// stored timestamp or at the current time, whichever is later, as Windows
/// Settings does.
pub(crate) fn write_settings_previewing(previewing: bool) -> anyhow::Result<u64> {
    let now = unix_secs(SystemTime::now())?;
    let mut stamped = 0;
    patch_settings(|envelope| {
        envelope.set_previewing(previewing)?;
        stamped = envelope
            .modified
            .checked_add(MODIFIED_STEP_SECS)
            .context("settings timestamp overflows")?
            .max(now);
        envelope.modified = stamped;
        Ok(())
    })?;
    Ok(stamped)
}

/// Turn preview on, wait for `gap`, then write `temperature` while preview
/// stays on, as the Night Light slider does.
pub(crate) fn start_preview(
    light: &Nightlight,
    temperature: ColorTemperature,
    gap: Duration,
) -> anyhow::Result<()> {
    write_settings_previewing(true)?;
    std::thread::sleep(gap);
    light.update_settings(|settings| settings.set_color_temperature(temperature))?;
    Ok(())
}

/// Change the temperature inside a preview: turn preview on, wait
/// [`PREVIEW_STEP`], write `temperature`, wait again, and turn preview off.
pub(crate) fn preview_refresh(
    light: &Nightlight,
    temperature: ColorTemperature,
) -> anyhow::Result<()> {
    start_preview(light, temperature, PREVIEW_STEP)?;
    std::thread::sleep(PREVIEW_STEP);
    write_settings_previewing(false)?;
    Ok(())
}

// The read and the write are not atomic, so a concurrent writer's change in
// between is lost.
fn patch_settings(
    change: impl FnOnce(&mut SettingsEnvelope) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let key = CURRENT_USER
        .options()
        .read()
        .write()
        .open(SETTINGS_KEY_PATH)
        .context("opening the Night Light settings key")?;
    let data = key
        .get_bytes(VALUE_NAME)
        .context("reading Night Light settings")?;
    let mut envelope = SettingsEnvelope::parse(&data)?;
    change(&mut envelope)?;
    key.set_bytes(VALUE_NAME, Type::Bytes, &envelope.encode()?)
        .context("writing Night Light settings")
}
