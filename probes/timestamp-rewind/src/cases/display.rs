//! Cases that need an observer to report whether the display follows
//! refreshes after a timestamp rewind or a related change.

use super::applied_or_ignored;
use super::restart_shell;
use crate::registry;
use crate::session::Session;
use crate::session::Tint;
use crate::session::say;
use crate::shell::ShellProcess;
use crate::snapshot::Snapshot;
use crate::snapshot::unix_secs;
use anyhow::Context as _;
use anyhow::ensure;
use serde_json::json;
use std::time::Duration;
use std::time::SystemTime;
use win_nightlight_lib::ColorTemperature;

const REVERT_CHECK_DELAY: Duration = Duration::from_millis(500);
const RESET_BURST: u32 = 64;
const CLOUD_LOWERING_SECS: u64 = 600;
const CONTRAST_THRESHOLD_KELVIN: i16 = 3800;

/// Advance the timestamp with a visible burst, rewind it to the later of real
/// time and `Cloud`, which Windows keeps, then make a red ordinary refresh
/// below the burst peak and ask whether it applied.
///
/// Returns the burst's highest timestamp.
fn burst_and_rewind(session: &mut Session, burst: u32) -> anyhow::Result<u64> {
    session.require_working_display()?;
    let peak = session.burst(Tint::Yellow, burst)?;
    ensure!(
        session.display_shows(Tint::Yellow, "burst")?,
        "the display did not follow the burst"
    );
    let target = unix_secs(SystemTime::now())?.max(peak.cloud_modified);
    rewind_settings(session, target)?;
    session.refresh(Tint::Red, "refresh_below_peak")?;
    let applied = session.display_shows(Tint::Red, "refresh_below_peak")?;
    session.record("refresh_below_peak", json!({"applied": applied}))?;
    say(&format!(
        "Refresh after the rewind to {target}, below the burst peak {}: {}",
        peak.settings_modified,
        applied_or_ignored(applied)
    ))?;
    Ok(peak.settings_modified)
}

pub(super) fn rewind_settings(session: &mut Session, target: u64) -> anyhow::Result<Snapshot> {
    registry::write_settings_modified(target)?;
    std::thread::sleep(REVERT_CHECK_DELAY);
    let rewound = session.snapshot("rewound")?;
    ensure!(
        rewound.settings_modified == target,
        "Windows replaced the rewind to {target} with {}",
        rewound.settings_modified
    );
    Ok(rewound)
}

pub(super) fn refresh_above(session: &mut Session, peak: u64, tint: Tint) -> anyhow::Result<bool> {
    registry::write_settings_modified(peak.checked_add(1).context("timestamp overflows")?)?;
    session.refresh(tint, "refresh_above_peak")?;
    let applied = session.display_shows(tint, "refresh_above_peak")?;
    say(&format!(
        "Refresh stamped above the burst peak {peak}: {}",
        applied_or_ignored(applied)
    ))?;
    Ok(applied)
}

pub(super) fn revert_below_cloud(session: &mut Session) -> anyhow::Result<Snapshot> {
    let cloud = registry::read_cloud_modified()?;
    registry::write_settings_modified(cloud.checked_sub(1).context("Cloud timestamp is zero")?)?;
    std::thread::sleep(REVERT_CHECK_DELAY);
    let reverted = session.snapshot("reverted")?;
    ensure!(
        reverted.settings_modified == cloud,
        "Windows did not revert the write below Cloud {cloud}"
    );
    Ok(reverted)
}

/// Check whether the display follows refreshes stamped below the timestamp
/// it applied before a rewind, and above it.
pub(crate) fn stale_timestamp(session: &mut Session, burst: u32) -> anyhow::Result<()> {
    let peak = burst_and_rewind(session, burst)?;
    refresh_above(session, peak, Tint::Neutral)?;
    Ok(())
}

/// Check whether a reverted write releases a stale-timestamp block.
pub(crate) fn revert_after_rewind(session: &mut Session, burst: u32) -> anyhow::Result<()> {
    let peak = burst_and_rewind(session, burst)?;
    let reverted = revert_below_cloud(session)?;
    // The revert restores the last Windows Settings temperature, which the
    // display may apply, so the check refreshes to a contrasting tint.
    let tint = if reverted
        .kelvin
        .is_some_and(|kelvin| kelvin > CONTRAST_THRESHOLD_KELVIN)
    {
        Tint::Red
    } else {
        Tint::Neutral
    };
    session.refresh(tint, "refresh_after_revert")?;
    let applied = session.display_shows(tint, "refresh_after_revert")?;
    say(&format!(
        "Refresh after the revert, still below the burst peak {peak}: {}",
        applied_or_ignored(applied)
    ))?;
    Ok(())
}

/// Check whether a held-preview refresh releases a stale-timestamp block that
/// a rewind caused before `explorer.exe` restarted.
///
/// An `explorer.exe` restart alone blocks ordinary refreshes, so the case
/// cannot tell whether the new process keeps the highest timestamp applied
/// before the restart. If the ordinary refresh after the held-preview refresh
/// is ignored, a refresh above the burst peak shows whether updates resumed
/// at all.
pub(crate) fn explorer_restart_after_rewind(
    session: &mut Session,
    burst: u32,
) -> anyhow::Result<()> {
    let peak = burst_and_rewind(session, burst)?;
    restart_shell(session, ShellProcess::Explorer)?;
    // The stored temperature is red from the ignored refresh, and the screen
    // still shows yellow from the burst unless the new explorer.exe applies the
    // stored temperature.
    let shown = if session.display_shows(Tint::Red, "after_explorer_restart")? {
        Tint::Red
    } else {
        Tint::Yellow
    };
    session.refresh(Tint::Neutral, "refresh_after_restart")?;
    let applied = session.display_shows(Tint::Neutral, "refresh_after_restart")?;
    say(&format!(
        "Ordinary refresh after the restart, below the burst peak {peak}: {}",
        applied_or_ignored(applied)
    ))?;
    if applied {
        return Ok(());
    }
    held_preview_refresh(session, super::contrasting(shown, Tint::Neutral))?;
    // Neutral differs from the held tint and from the tint shown before it.
    session.refresh(Tint::Neutral, "refresh_after_held_preview")?;
    let applied = session.display_shows(Tint::Neutral, "refresh_after_held_preview")?;
    say(&format!(
        "Ordinary refresh below the burst peak {peak} after the held-preview refresh: {}",
        applied_or_ignored(applied)
    ))?;
    if !applied {
        refresh_above(session, peak, Tint::Neutral)?;
    }
    Ok(())
}

/// Check whether the display follows refreshes after the `Cloud` timestamp is
/// lowered without changing the settings.
pub(crate) fn lower_cloud(session: &mut Session) -> anyhow::Result<()> {
    session.require_working_display()?;
    let cloud = registry::read_cloud_modified()?;
    let lowered = cloud.saturating_sub(CLOUD_LOWERING_SECS);
    registry::write_cloud_modified(lowered)?;
    session.record("cloud_lowered", json!({"from": cloud, "to": lowered}))?;
    session.refresh(Tint::Neutral, "refresh_after_lowering")?;
    let applied = session.display_shows(Tint::Neutral, "refresh_after_lowering")?;
    say(&format!(
        "Refresh after lowering Cloud from {cloud} to {lowered}: {}",
        applied_or_ignored(applied)
    ))?;
    Ok(())
}

/// Check whether the display follows refreshes after both the `Cloud` and the
/// settings timestamps are set to real time.
pub(crate) fn reset_both(session: &mut Session) -> anyhow::Result<()> {
    session.require_working_display()?;
    let peak = session.burst(Tint::Neutral, RESET_BURST)?;
    let target = unix_secs(SystemTime::now())?;
    ensure!(
        target < peak.settings_modified,
        "the burst did not advance the settings timestamp past real time"
    );
    registry::write_cloud_modified(target)?;
    rewind_settings(session, target)?;
    session.refresh(Tint::Red, "refresh_after_reset")?;
    let applied = session.display_shows(Tint::Red, "refresh_after_reset")?;
    say(&format!(
        "Refresh after setting Cloud and the settings to {target}, below the burst peak {}: {}",
        peak.settings_modified,
        applied_or_ignored(applied)
    ))?;
    // Yellow differs from both possible screens: neutral from the burst if the
    // red refresh was ignored, or red if it applied.
    refresh_above(session, peak.settings_modified, Tint::Yellow)?;
    Ok(())
}

/// Restart `process` while the screen is red, ask whether the screen changed
/// by itself, then check whether a refresh to neutral applies.
///
/// Returns whether the refresh applied.
fn restart_then_refresh(session: &mut Session, process: ShellProcess) -> anyhow::Result<bool> {
    session.require_working_display()?;
    restart_shell(session, process)?;
    session.display_shows(Tint::Red, "after_restart")?;
    session.refresh(Tint::Neutral, "refresh_after_restart")?;
    let applied = session.display_shows(Tint::Neutral, "refresh_after_restart")?;
    say(&format!(
        "Refresh after restarting {}: {}",
        process.name(),
        applied_or_ignored(applied)
    ))?;
    Ok(applied)
}

/// Check whether the display follows refreshes after `ShellHost.exe`
/// restarts.
pub(crate) fn restart_shellhost(session: &mut Session) -> anyhow::Result<()> {
    restart_then_refresh(session, ShellProcess::ShellHost)?;
    Ok(())
}

/// Ask the observer to open the Night Light page without changing anything,
/// then check whether a refresh to `tint` applies.
///
/// The screen still shows `shown`, and the stored temperature may differ from
/// it, so the case first asks whether opening the page changed the screen.
/// `tint` must differ from both.
fn open_settings_then_refresh(
    session: &mut Session,
    shown: Tint,
    tint: Tint,
) -> anyhow::Result<bool> {
    session.wait_for_observer(
        "Open Settings > System > Display > Night light without touching any control, wait \
         until the page shows, then close Settings.",
    )?;
    let opened = session.snapshot("after_opening_settings")?;
    session.display_shows(shown, "after_opening_settings")?;
    session.refresh(tint, "refresh_after_opening_settings")?;
    let applied = session.display_shows(tint, "refresh_after_opening_settings")?;
    say(&format!(
        "Refresh stamped {} after opening Settings: {}",
        opened.settings_modified,
        applied_or_ignored(applied)
    ))?;
    Ok(applied)
}

/// Check whether the display follows refreshes after `explorer.exe`
/// restarts, and whether opening or using the Night Light page in Windows
/// Settings afterwards restores them.
pub(crate) fn restart_explorer(session: &mut Session) -> anyhow::Result<()> {
    if !restart_then_refresh(session, ShellProcess::Explorer)?
        && !open_settings_then_refresh(session, Tint::Red, Tint::Yellow)?
    {
        refresh_after_slider(session)?;
    }
    Ok(())
}

/// Ask the observer to use the Night Light slider and release it at 6500 K,
/// then check whether a refresh to red applies.
fn refresh_after_slider(session: &mut Session) -> anyhow::Result<bool> {
    session.wait_for_observer(
        "Open Settings > System > Display > Night light, drag the Strength slider, release it \
         at 0, and close Settings.",
    )?;
    session.snapshot("after_slider")?;
    session.refresh(Tint::Red, "refresh_after_slider")?;
    let applied = session.display_shows(Tint::Red, "refresh_after_slider")?;
    say(&format!(
        "Refresh after using the slider: {}",
        applied_or_ignored(applied)
    ))?;
    Ok(applied)
}

/// Request a refresh to `tint` that keeps preview on until the observer
/// answers, then turn preview off.
///
/// Returns whether the screen still shows `tint` after preview turns off.
fn held_preview_refresh(session: &mut Session, tint: Tint) -> anyhow::Result<bool> {
    registry::start_preview(
        &session.light,
        ColorTemperature::new(tint.kelvin())?,
        registry::PREVIEW_STEP,
    )?;
    session.snapshot("held_preview_on")?;
    say("Preview is on and stays on until you answer.")?;
    let while_previewing = session.display_shows(tint, "held_preview_on")?;
    registry::write_settings_previewing(false)?;
    session.snapshot("held_preview_off")?;
    say("Preview is now off.")?;
    let after_preview = session.display_shows(tint, "held_preview_off")?;
    say(&format!(
        "Held-preview refresh: {} while preview was on, {} after it turned off",
        applied_or_ignored(while_previewing),
        if after_preview { "kept" } else { "not kept" }
    ))?;
    Ok(after_preview)
}

/// Check whether a refresh that holds preview on, like the Night Light slider,
/// applies after an `explorer.exe` restart blocks ordinary refreshes, and
/// whether ordinary refreshes apply after it.
pub(crate) fn held_preview_after_explorer_restart(session: &mut Session) -> anyhow::Result<()> {
    if restart_then_refresh(session, ShellProcess::Explorer)? {
        return say(
            "The explorer.exe restart did not block refreshes this time, so the held-preview \
             check does not apply.",
        );
    }
    held_preview_refresh(session, Tint::Yellow)?;
    // Neutral differs from yellow, if the held refresh applied, and from red,
    // if it did not.
    session.refresh(Tint::Neutral, "refresh_after_held_preview")?;
    let applied = session.display_shows(Tint::Neutral, "refresh_after_held_preview")?;
    say(&format!(
        "Ordinary refresh after the held-preview refresh: {}",
        applied_or_ignored(applied)
    ))?;
    if !applied {
        refresh_after_slider(session)?;
    }
    Ok(())
}

/// Check whether a refresh that holds preview on applies during a
/// stale-timestamp block, and whether ordinary refreshes below the burst peak
/// apply after it.
pub(crate) fn held_preview_after_rewind(session: &mut Session, burst: u32) -> anyhow::Result<()> {
    let peak = burst_and_rewind(session, burst)?;
    // The screen shows yellow from the burst; the red refresh was ignored.
    let held_kept = held_preview_refresh(session, Tint::Neutral)?;
    // Red differs from neutral, if the held refresh applied, and from yellow,
    // if it did not.
    session.refresh(Tint::Red, "refresh_after_held_preview")?;
    let applied = session.display_shows(Tint::Red, "refresh_after_held_preview")?;
    say(&format!(
        "Ordinary refresh below the burst peak {peak} after the held-preview refresh: {}",
        applied_or_ignored(applied)
    ))?;
    if !applied {
        refresh_above(
            session,
            peak,
            if held_kept { Tint::Yellow } else { Tint::Red },
        )?;
    }
    Ok(())
}
