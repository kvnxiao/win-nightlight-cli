//! A case in which the observer moves the Night Light slider and switches
//! Night Light in Windows, so a trace shows how Windows stamps those writes.

use super::drag::sleep_until;
use crate::session::Session;
use crate::session::say;
use crate::snapshot::modified_secs;
use crate::snapshot::unix_secs;
use anyhow::Context as _;
use serde_json::json;
use std::time::Duration;
use std::time::Instant;

const WATCHED_DRAGS: u32 = 2;
const REWRITES: u32 = 10;
const REWRITE_INTERVAL: Duration = Duration::from_secs(1);
const PRESS_LEAD: Duration = Duration::from_secs(3);
const SWITCHES_PER_PLACE: u32 = 2;
const SWITCH_PLACES: [(&str, &str); 2] = [
    (
        "quick_settings",
        "Open Quick Settings with Windows+A, switch Night light off and then on, and close Quick \
         Settings.",
    ),
    (
        "settings",
        "On the Night light page in Windows Settings, press Turn off now, then Turn on now.",
    ),
];

/// Watch two slider drags, write the stored settings back unchanged once per
/// second during a third drag, and watch Night Light switched off and on in
/// Quick Settings and in Windows Settings.
///
/// Run it under `just trace`: the trace shows how Windows Settings stamps
/// slider writes with and without another writer, whether a release turns
/// preview off, and how each switch stamps the state value.
pub(crate) fn slider_stamps(session: &mut Session) -> anyhow::Result<()> {
    session
        .wait_for_observer("Open Settings > System > Display > Night light and keep it open.")?;
    let mut preview_after_release = Vec::new();
    for drag in 1..=WATCHED_DRAGS {
        session.snapshot("watched_drag_start")?;
        session.wait_for_observer(
            "Press the Strength slider, drag it slowly back and forth for about 3 s, and \
             release it.",
        )?;
        let released = session.snapshot("watched_drag_end")?;
        session.record(
            "watched_drag",
            json!({"drag": drag, "previewing_after_release": released.previewing}),
        )?;
        preview_after_release.push(released.previewing);
    }
    preview_after_release.push(drag_during_rewrites(session)?);
    for (place, instruction) in SWITCH_PLACES {
        for switch in 1..=SWITCHES_PER_PLACE {
            session.wait_for_observer(instruction)?;
            let state = session.light.read_state()?;
            let modified = state
                .modified()
                .context("stored state timestamp is out of range")
                .and_then(unix_secs)?;
            session.record(
                "switch",
                json!({
                    "place": place,
                    "switch": switch,
                    "active": state.is_active(),
                    "state_modified_unix_s": modified,
                }),
            )?;
        }
    }
    session.wait_for_observer("Close Settings.")?;
    say(&format!(
        "Preview after each slider release, in order: {preview_after_release:?}"
    ))
}

/// Write the stored settings back unchanged [`REWRITES`] times, once per
/// [`REWRITE_INTERVAL`], while the observer drags the slider, and return
/// whether preview was on after the release.
///
/// Each write advances the settings timestamp without changing the
/// temperature or preview, so the next slider write shows whether Windows
/// Settings stamped it from this write or from its own previous write.
fn drag_during_rewrites(session: &mut Session) -> anyhow::Result<bool> {
    session.wait_for_observer(&format!(
        "Next, the probe writes the stored settings back unchanged once per second for {} s. \
         After you press Enter, press the Strength slider and keep dragging it slowly back and \
         forth until the probe asks you to release it.",
        REWRITE_INTERVAL.as_secs() * u64::from(REWRITES)
    ))?;
    say(&format!(
        "Press the Strength slider now. The probe starts writing in {} s.",
        PRESS_LEAD.as_secs()
    ))?;
    std::thread::sleep(PRESS_LEAD);
    let started = Instant::now();
    let mut rewrites = Vec::new();
    for index in 1..=REWRITES {
        session.check_quit()?;
        sleep_until(started + REWRITE_INTERVAL * index);
        let settings = session.light.read_settings()?;
        session.light.write_settings(&settings)?;
        let read_back = session.light.read_settings()?;
        rewrites.push(json!({
            "offset_ms": u64::try_from(started.elapsed().as_millis())?,
            "stored_before_unix_s": modified_secs(&settings)?,
            "read_back_unix_s": modified_secs(&read_back)?,
            "previewing": settings.is_previewing(),
        }));
    }
    session.record("rewrites", json!({"rewrites": rewrites}))?;
    session.wait_for_observer("Release the slider.")?;
    let released = session.snapshot("rewrite_drag_end")?;
    session.record(
        "rewrite_drag",
        json!({"previewing_after_release": released.previewing}),
    )?;
    Ok(released.previewing)
}
