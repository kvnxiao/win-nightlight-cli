//! Cases that write the settings below the `Cloud` timestamp and measure how
//! Windows replaces them. They do not need an observer.

use crate::registry;
use crate::session::Session;
use crate::session::say;
use crate::snapshot;
use anyhow::Context as _;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use win_nightlight_lib::ColorTemperature;

const REVERT_WATCH: Duration = Duration::from_secs(1);
const REVERT_POLL: Duration = Duration::from_millis(1);
const REFRESH_DELAYS_MS: [u64; 14] = [0, 1, 2, 3, 5, 8, 12, 20, 30, 40, 50, 60, 80, 120];
const REFRESH_KELVINS: [u16; 2] = [3000, 4000];
const SETTLE: Duration = Duration::from_millis(300);

/// Write the settings one second below `Cloud` and record every change in the
/// following second.
pub(crate) fn below_cloud(session: &mut Session) -> anyhow::Result<()> {
    let before = session.snapshot("before")?;
    let target = before
        .cloud_modified
        .checked_sub(1)
        .context("Cloud timestamp is zero")?;
    registry::write_settings_modified(target)?;
    let changes = snapshot::changes(&session.light, REVERT_WATCH, REVERT_POLL)?;
    say(&format!(
        "Wrote settings at {target}, one second below Cloud {}. Changes in the next {} ms:",
        before.cloud_modified,
        REVERT_WATCH.as_millis()
    ))?;
    for change in &changes {
        say(&change.to_string())?;
    }
    session.record(
        "write_below_cloud",
        json!({"written_modified_unix_s": target, "changes": changes}),
    )?;
    Ok(())
}

/// Write the settings exactly at `Cloud`, then one second below it, and
/// record whether Windows keeps each write.
pub(crate) fn cloud_boundary(session: &mut Session) -> anyhow::Result<()> {
    for offset in [0, -1] {
        let cloud = registry::read_cloud_modified()?;
        let target = cloud
            .checked_add_signed(offset)
            .context("Cloud timestamp is out of range")?;
        registry::write_settings_modified(target)?;
        let changes = snapshot::changes(&session.light, REVERT_WATCH, REVERT_POLL)?;
        let kept = changes
            .iter()
            .all(|change| change.get("settings_modified_unix_s") == Some(&json!(target)));
        say(&format!(
            "Settings written at Cloud {offset:+} s ({target}): {}",
            if kept { "kept" } else { "replaced" }
        ))?;
        session.record(
            "write_at_cloud_offset",
            json!({"offset_s": offset, "written_modified_unix_s": target, "kept": kept, "changes": changes}),
        )?;
    }
    Ok(())
}

/// Repeatedly write the settings below `Cloud`, make an ordinary refresh
/// after a growing delay, and count the refreshes Windows overwrites.
pub(crate) fn refresh_window(session: &mut Session, passes: usize) -> anyhow::Result<()> {
    let mut lost: BTreeMap<u64, (usize, usize)> = BTreeMap::new();
    let rounds = REFRESH_DELAYS_MS
        .into_iter()
        .cycle()
        .take(REFRESH_DELAYS_MS.len() * passes);
    for (kelvin, delay_ms) in REFRESH_KELVINS.into_iter().cycle().zip(rounds) {
        session.check_quit()?;
        let target = registry::read_cloud_modified()?
            .checked_sub(1)
            .context("Cloud timestamp is zero")?;
        registry::write_settings_modified(target)?;
        std::thread::sleep(Duration::from_millis(delay_ms));
        registry::ordinary_refresh(&session.light, ColorTemperature::new(kelvin)?)?;
        std::thread::sleep(SETTLE);
        let settled = session.snapshot("round")?;
        let kept = settled.kelvin == Some(kelvin.cast_signed());
        session.record(
            "refresh_after_revert",
            json!({"delay_ms": delay_ms, "requested_kelvin": kelvin, "settled_kelvin": settled.kelvin, "kept": kept}),
        )?;
        let (total, overwritten) = lost.entry(delay_ms).or_default();
        *total += 1;
        *overwritten += usize::from(!kept);
    }
    for (delay_ms, (total, overwritten)) in &lost {
        say(&format!(
            "refresh {delay_ms:>3} ms after the write: {overwritten}/{total} overwritten"
        ))?;
    }
    session.record("summary", json!(lost))?;
    Ok(())
}
