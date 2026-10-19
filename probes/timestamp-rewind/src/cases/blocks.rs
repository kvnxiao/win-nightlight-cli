//! Cases that block ordinary refreshes, with a rewind or an `explorer.exe`
//! restart, and check what applies a temperature or releases the block.

use super::applied_or_ignored;
use super::contrasting;
use super::display::refresh_above;
use super::display::revert_below_cloud;
use super::display::rewind_settings;
use super::restart_shell;
use crate::registry;
use crate::session::Session;
use crate::session::Tint;
use crate::session::say;
use crate::shell::ShellProcess;
use crate::snapshot::unix_secs;
use anyhow::Context as _;
use anyhow::ensure;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use win_nightlight_lib::ColorTemperature;

// Each trial writes about ten seconds of timestamps after its rewind, and all
// of them must stay below the highest timestamp the display applied.
const REWIND_GAP_SECS: u64 = 60;
// Each pair is the time between turning preview on and changing the
// temperature, then the time between that change and turning preview off.
const PREVIEW_TIMINGS_MS: [(u64, u64); 4] = [(0, 0), (0, 50), (50, 0), (50, 50)];
const BLOCK_NOT_CONFIRMED: &str = "the screen was not red with ordinary refreshes ignored";
// Each refresh advances the stored timestamp by 4 s, so this leaves about
// 64 s for the observer and both reverts before real time passes Cloud.
const CLOUD_LEAD_BURST: u32 = 16;

/// What blocks ordinary refreshes before each trial.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub(crate) enum Blocker {
    /// Nothing; ordinary refreshes apply.
    None,
    /// A rewind below the highest timestamp the display applied.
    StaleTimestamp,
    /// An explorer.exe restart.
    ExplorerRestart,
}

/// What the display did after a preview refresh with set timings.
#[derive(Debug, Clone, Copy)]
enum TrialOutcome {
    NotBlocked,
    Ignored,
    AppliedWithoutRelease,
    Released,
}

impl TrialOutcome {
    fn description(self) -> &'static str {
        match self {
            Self::NotBlocked => "skipped, the screen was not red with refreshes ignored",
            Self::Ignored => "ignored",
            Self::AppliedWithoutRelease => "applied, later ordinary refreshes still ignored",
            Self::Released => "applied, later ordinary refreshes applied",
        }
    }
}

/// Leave the screen red with ordinary refreshes blocked by `blocker`, then
/// ask the observer to confirm.
///
/// Afterwards the stored temperature is yellow, or red for
/// [`Blocker::None`].
pub(super) fn block_refreshes(session: &mut Session, blocker: Blocker) -> anyhow::Result<bool> {
    match blocker {
        Blocker::None => {
            session.refresh(Tint::Red, "block")?;
        }
        Blocker::StaleTimestamp => {
            let floor = unix_secs(SystemTime::now())?.max(registry::read_cloud_modified()?);
            // The red refresh must apply even when an earlier trial left a
            // stale-timestamp block, so it is stamped above every timestamp the
            // session has seen.
            let above_seen = session
                .highest_modified()
                .checked_add(1)
                .context("settings timestamp overflows")?;
            let above_floor = floor
                .checked_add(REWIND_GAP_SECS)
                .context("settings timestamp overflows")?;
            registry::write_settings_modified(above_seen.max(above_floor))?;
            session.refresh(Tint::Red, "block")?;
            rewind_settings(session, floor)?;
        }
        Blocker::ExplorerRestart => {
            session.refresh(Tint::Red, "block")?;
            restart_shell(session, ShellProcess::Explorer)?;
        }
    }
    if !matches!(blocker, Blocker::None) {
        // The screen stays red only if it was red and this refresh is ignored.
        session.refresh(Tint::Yellow, "blocked_check")?;
    }
    session.display_shows(Tint::Red, "blocked_check")
}

/// Turn preview on, change the temperature to neutral after `gap`, and turn
/// preview off after `hold`, then check whether an ordinary refresh to yellow
/// applies after it.
fn timed_preview_trial(
    session: &mut Session,
    gap: Duration,
    hold: Duration,
) -> anyhow::Result<TrialOutcome> {
    let temperature = ColorTemperature::new(Tint::Neutral.kelvin())?;
    let started = Instant::now();
    registry::start_preview(&session.light, temperature, gap)?;
    let changed = Instant::now();
    std::thread::sleep(hold);
    registry::write_settings_previewing(false)?;
    let ended = Instant::now();
    session.record(
        "timed_preview",
        json!({
            "gap_ms": u64::try_from(gap.as_millis())?,
            "hold_ms": u64::try_from(hold.as_millis())?,
            "measured_gap_us": u64::try_from(changed.duration_since(started).as_micros())?,
            "measured_hold_us": u64::try_from(ended.duration_since(changed).as_micros())?,
        }),
    )?;
    session.snapshot("timed_preview_off")?;
    if !session.display_shows(Tint::Neutral, "timed_preview")? {
        return Ok(TrialOutcome::Ignored);
    }
    session.refresh(Tint::Yellow, "refresh_after_timed_preview")?;
    Ok(
        if session.display_shows(Tint::Yellow, "refresh_after_timed_preview")? {
            TrialOutcome::Released
        } else {
            TrialOutcome::AppliedWithoutRelease
        },
    )
}

/// Parse a preview timing written as `<gap ms>/<hold ms>`.
///
/// # Errors
///
/// Returns an error if `text` is not two whole numbers separated by `/`.
pub(crate) fn parse_timing(text: &str) -> anyhow::Result<(u64, u64)> {
    let (gap, hold) = text
        .split_once('/')
        .context("expected <gap ms>/<hold ms>, such as 0/0")?;
    Ok((gap.parse()?, hold.parse()?))
}

/// Check which preview timings apply a temperature while `blocker` blocks
/// ordinary refreshes, and whether ordinary refreshes apply afterwards.
///
/// Each round tries `timing`, or every timing in [`PREVIEW_TIMINGS_MS`] when
/// it is `None`. Each trial blocks refreshes again and checks the block, so
/// trials do not depend on each other.
pub(crate) fn preview_timing(
    session: &mut Session,
    blocker: Blocker,
    rounds: u32,
    timing: Option<(u64, u64)>,
) -> anyhow::Result<()> {
    session.require_working_display()?;
    let mut outcomes: BTreeMap<(u64, u64), Vec<&str>> = BTreeMap::new();
    for round in 1..=rounds {
        let mut timings = timing.map_or_else(|| PREVIEW_TIMINGS_MS.to_vec(), |only| vec![only]);
        // Alternating the order keeps a change over the run, such as observer
        // fatigue, from looking like an effect of the timing.
        if round.is_multiple_of(2) {
            timings.reverse();
        }
        for (gap_ms, hold_ms) in timings {
            say(&format!(
                "Round {round} of {rounds}, trial with a {gap_ms} ms gap and a {hold_ms} ms hold."
            ))?;
            let outcome = if block_refreshes(session, blocker)? {
                timed_preview_trial(
                    session,
                    Duration::from_millis(gap_ms),
                    Duration::from_millis(hold_ms),
                )?
            } else {
                TrialOutcome::NotBlocked
            };
            session.record(
                "trial_outcome",
                json!({
                    "round": round,
                    "gap_ms": gap_ms,
                    "hold_ms": hold_ms,
                    "outcome": outcome.description(),
                }),
            )?;
            say(&format!(
                "{gap_ms} ms gap, {hold_ms} ms hold: {}",
                outcome.description()
            ))?;
            outcomes
                .entry((gap_ms, hold_ms))
                .or_default()
                .push(outcome.description());
        }
    }
    let summary: Vec<String> = outcomes
        .iter()
        .map(|((gap_ms, hold_ms), results)| {
            format!("{gap_ms} ms gap, {hold_ms} ms hold: {}", results.join("; "))
        })
        .collect();
    say(&format!("Preview timing results:\n{}", summary.join("\n")))
}

/// Switch Night Light off, ask the observer whether the screen is neutral, and
/// switch it back on.
///
/// Returns whether the screen then shows `stored`, the stored temperature.
fn switch_off_and_on(session: &mut Session, stored: Tint) -> anyhow::Result<bool> {
    session.light.set_active(false)?;
    session.snapshot("switched_off")?;
    session.display_shows(Tint::Neutral, "switched_off")?;
    session.light.set_active(true)?;
    session.snapshot("switched_on")?;
    session.display_shows(stored, "switched_on")
}

/// Check whether switching Night Light off and on applies the stored
/// temperature while `blocker` blocks ordinary refreshes, and whether ordinary
/// refreshes apply afterwards.
pub(crate) fn switch_release(session: &mut Session, blocker: Blocker) -> anyhow::Result<()> {
    session.require_working_display()?;
    ensure!(block_refreshes(session, blocker)?, BLOCK_NOT_CONFIRMED);
    let applied = switch_off_and_on(session, Tint::Yellow)?;
    say(&format!(
        "Stored yellow temperature after switching off and on: {}",
        applied_or_ignored(applied)
    ))?;
    session.refresh(Tint::Neutral, "refresh_after_switching")?;
    let applied = session.display_shows(Tint::Neutral, "refresh_after_switching")?;
    say(&format!(
        "Ordinary refresh after switching off and on: {}",
        applied_or_ignored(applied)
    ))
}

/// Check whether the highest timestamp the display applied before an
/// `explorer.exe` restart still blocks refreshes after it.
///
/// A preview refresh releases a restart block but not a stale-timestamp block,
/// so a preview refresh that applies after the restart shows that the
/// stale-timestamp block did not survive. An ordinary refresh below the old
/// maximum then confirms it.
pub(crate) fn stale_timestamp_survives_restart(session: &mut Session) -> anyhow::Result<()> {
    session.require_working_display()?;
    ensure!(
        block_refreshes(session, Blocker::StaleTimestamp)?,
        BLOCK_NOT_CONFIRMED
    );
    let peak = session.highest_modified();
    restart_shell(session, ShellProcess::Explorer)?;
    // The stored temperature is yellow from the block check, which the new
    // explorer.exe may apply at startup.
    let shown = if session.display_shows(Tint::Yellow, "after_explorer_restart")? {
        Tint::Yellow
    } else {
        Tint::Red
    };
    session.preview_refresh(Tint::Neutral, "preview_refresh_after_restart")?;
    let previewed = session.display_shows(Tint::Neutral, "preview_refresh_after_restart")?;
    say(&format!(
        "Preview refresh after the restart, below the old maximum {peak}: {}",
        applied_or_ignored(previewed)
    ))?;
    if !previewed {
        refresh_above(session, peak, contrasting(shown, Tint::Neutral))?;
        return Ok(());
    }
    session.refresh(Tint::Red, "refresh_below_old_peak")?;
    let applied = session.display_shows(Tint::Red, "refresh_below_old_peak")?;
    say(&format!(
        "Ordinary refresh below the old maximum {peak}: {}",
        applied_or_ignored(applied)
    ))?;
    if !applied {
        refresh_above(session, peak, Tint::Yellow)?;
    }
    Ok(())
}

/// Twice write below `Cloud`, which Windows reverts, refresh, and switch Night
/// Light off, so the second write is made with Night Light off. Then switch it
/// on and check whether a refresh below the burst peak and one above it apply.
///
/// With `cloud_ahead`, a slider drag first moves `Cloud` ahead of real time,
/// so the refreshes after both reverts reuse the same timestamps, and the
/// case waits for real time to pass them before switching on. If
/// the refresh above the peak is ignored, the case tries each known release
/// in turn and stops at the first that works.
pub(crate) fn reverts_while_off(
    session: &mut Session,
    burst: u32,
    cloud_ahead: bool,
) -> anyhow::Result<()> {
    session.require_working_display()?;
    if cloud_ahead {
        // Windows Settings stamps each slider write above the stored
        // timestamp, so after a burst the slider leaves Cloud ahead of real
        // time. explorer.exe reverts to its own copy of the last Windows
        // Settings write, so a real slider write sets both that copy and
        // Cloud.
        session.burst(Tint::Neutral, CLOUD_LEAD_BURST)?;
        session.wait_for_observer(
            "Open Settings > System > Display > Night light, drag the Strength slider, release \
             it anywhere, and close Settings.",
        )?;
        let cloud = session.snapshot("after_slider")?.cloud_modified;
        ensure!(
            cloud > unix_secs(SystemTime::now())?,
            "the slider did not move Cloud ahead of real time"
        );
    }
    let peak = session.burst(Tint::Yellow, burst)?;
    ensure!(
        session.display_shows(Tint::Yellow, "burst")?,
        "the display did not follow the burst"
    );
    for _ in 0..2 {
        revert_below_cloud(session)?;
        session.refresh(Tint::Red, "refresh_after_revert")?;
        session.light.set_active(false)?;
        session.snapshot("switched_off")?;
    }
    if cloud_ahead {
        let stored = session.snapshot("before_wait")?.settings_modified;
        say(&format!(
            "Waiting {} s until real time passes the stored timestamp {stored}.",
            stored.saturating_sub(unix_secs(SystemTime::now())?)
        ))?;
        while unix_secs(SystemTime::now())? <= stored {
            session.check_quit()?;
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    session.light.set_active(true)?;
    session.snapshot("switched_on")?;
    session.display_shows(Tint::Red, "switched_on")?;
    session.refresh(Tint::Neutral, "refresh_below_peak")?;
    let applied = session.display_shows(Tint::Neutral, "refresh_below_peak")?;
    say(&format!(
        "Refresh below the burst peak {}: {}",
        peak.settings_modified,
        applied_or_ignored(applied)
    ))?;
    if refresh_above(session, peak.settings_modified, Tint::Yellow)? {
        return say("The display followed the refresh above the burst peak.");
    }
    let mut shown = if applied { Tint::Neutral } else { Tint::Red };
    let mut stored = Tint::Yellow;
    let tint = contrasting(shown, stored);
    session.preview_refresh(tint, "preview_refresh")?;
    if session.display_shows(tint, "preview_refresh")? {
        return say("A preview refresh released the display.");
    }
    stored = tint;
    if switch_off_and_on(session, stored)? {
        shown = stored;
    }
    let tint = contrasting(shown, stored);
    session.refresh(tint, "refresh_after_switching")?;
    if session.display_shows(tint, "refresh_after_switching")? {
        return say("Switching off and on released the display.");
    }
    session.wait_for_observer(
        "Open Settings > System > Display > Night light, drag the Strength slider, release it \
         at 100, and close Settings.",
    )?;
    session.snapshot("after_slider")?;
    // Strength 100 is the warmest temperature, which matches deep red.
    stored = Tint::Red;
    if session.display_shows(stored, "after_slider")? {
        shown = stored;
    }
    let tint = contrasting(shown, stored);
    session.refresh(tint, "refresh_after_slider")?;
    if session.display_shows(tint, "refresh_after_slider")? {
        return say("Using the slider released the display.");
    }
    restart_shell(session, ShellProcess::Explorer)?;
    // The new explorer.exe applies the stored temperature at startup.
    let tint = contrasting(tint, tint);
    session.preview_refresh(tint, "preview_refresh_after_restart")?;
    if session.display_shows(tint, "preview_refresh_after_restart")? {
        return say("An explorer.exe restart and a preview refresh released the display.");
    }
    say("No release worked; sign out and back in to recover Night Light.")
}
