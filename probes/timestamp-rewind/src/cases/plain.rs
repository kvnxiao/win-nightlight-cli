//! Cases that write the temperature without preview, as
//! `Nightlight::set_color_temperature` does, and ask whether the display
//! applies it.

use super::blocks::Blocker;
use super::blocks::block_refreshes;
use super::contrasting;
use super::drag::ask_fade_outcome;
use super::drag::count_down;
use super::drag::stepped_fade;
use super::restart_shell;
use crate::session::Session;
use crate::session::Tint;
use crate::session::say;
use crate::shell::ShellProcess;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use win_nightlight_lib::ColorTemperature;

/// The display's condition before a plain write.
#[derive(Debug, Clone, Copy)]
enum Condition {
    Normal,
    AfterSwitchingOffAndOn,
    AfterExplorerRestart,
    StaleTimestamp,
}

impl Condition {
    fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::AfterSwitchingOffAndOn => "after_switching_off_and_on",
            Self::AfterExplorerRestart => "after_explorer_restart",
            Self::StaleTimestamp => "stale_timestamp",
        }
    }
}

/// Put the display in `condition` and return the tint on screen and the
/// stored tint, or `None` if the observer did not confirm a stale-timestamp
/// block.
fn prepare(
    session: &mut Session,
    condition: Condition,
    start: Tint,
) -> anyhow::Result<Option<(Tint, Tint)>> {
    if matches!(condition, Condition::StaleTimestamp) {
        // The block check leaves the screen red and stores yellow.
        return Ok(
            block_refreshes(session, Blocker::StaleTimestamp)?.then_some((Tint::Red, Tint::Yellow))
        );
    }
    session.refresh(start, "plain_setup")?;
    match condition {
        Condition::AfterSwitchingOffAndOn => {
            session.light.set_active(false)?;
            session.light.set_active(true)?;
            session.snapshot("switched_on")?;
        }
        // The new explorer.exe applies the stored temperature at startup,
        // even when an earlier restart left ordinary refreshes blocked.
        Condition::AfterExplorerRestart => restart_shell(session, ShellProcess::Explorer)?,
        Condition::Normal | Condition::StaleTimestamp => {}
    }
    Ok(Some((start, start)))
}

/// Write a tint other than the one on screen and the stored one, without
/// preview, and ask whether the display applied it.
fn plain_trial(
    session: &mut Session,
    condition: Condition,
    trial: u32,
) -> anyhow::Result<&'static str> {
    let start = if trial.is_multiple_of(2) {
        Tint::Red
    } else {
        Tint::Neutral
    };
    let Some((shown, stored)) = prepare(session, condition, start)? else {
        return Ok("not_blocked");
    };
    let tint = contrasting(shown, stored);
    let temperature = ColorTemperature::new(tint.kelvin())?;
    session.light.set_color_temperature(temperature)?;
    session.snapshot("plain_write")?;
    Ok(if session.display_shows(tint, "plain_write")? {
        "applied"
    } else {
        "ignored"
    })
}

/// Fade from neutral to red with writes without preview 3 s apart, then make
/// `trials` such writes for each display condition.
///
/// The fade runs first because the last condition leaves a stale-timestamp
/// block, during which Windows would ignore every write of the fade.
pub(crate) fn plain_write(session: &mut Session, trials: u32) -> anyhow::Result<()> {
    session.require_working_display()?;
    session.refresh(Tint::Neutral, "ramp_setup")?;
    count_down("the temperature changes from neutral to red every 3 s for 1 minute.")?;
    stepped_fade(session, "plain_ramp", |session, temperature| {
        let wrote = session.light.set_color_temperature(temperature)?;
        Ok(json!({"kelvin": temperature.kelvin(), "wrote": wrote}))
    })?;
    let (looked, ended_red) = ask_fade_outcome(session, "ramp_outcome", &Value::Null)?;
    let mut outcomes: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for condition in [
        Condition::Normal,
        Condition::AfterSwitchingOffAndOn,
        Condition::AfterExplorerRestart,
        Condition::StaleTimestamp,
    ] {
        for trial in 1..=trials {
            say(&format!(
                "Plain write, {}, trial {trial} of {trials}.",
                condition.name()
            ))?;
            let outcome = plain_trial(session, condition, trial)?;
            session.record(
                "plain_outcome",
                json!({"condition": condition.name(), "trial": trial, "outcome": outcome}),
            )?;
            outcomes.entry(condition.name()).or_default().push(outcome);
        }
        if matches!(condition, Condition::AfterExplorerRestart) {
            // A preview refresh, which keeps preview on for 50 ms before and 50
            // ms after the temperature write, releases the restart
            // block, so later conditions start from a display that
            // follows refreshes.
            session.preview_refresh(Tint::Neutral, "release_after_restarts")?;
        }
    }
    let summary: Vec<String> = outcomes
        .iter()
        .map(|(condition, results)| format!("{condition}: {}", results.join(", ")))
        .collect();
    say(&format!(
        "Plain write results:\n{}\nFade: looked {}, ended on red: {ended_red}",
        summary.join("\n"),
        looked.map_or("other", |looked| looked.name())
    ))
}

/// Repeat `trials` times: switch Night Light off, store, without preview, a
/// tint other than the one on screen, switch Night Light on, and ask whether
/// the screen shows the stored tint.
pub(crate) fn write_while_off(session: &mut Session, trials: u32) -> anyhow::Result<()> {
    session.require_working_display()?;
    let mut outcomes = Vec::new();
    for trial in 1..=trials {
        session.check_quit()?;
        let start = if trial.is_multiple_of(2) {
            Tint::Red
        } else {
            Tint::Neutral
        };
        session.refresh(start, "while_off_setup")?;
        session.light.set_active(false)?;
        let tint = contrasting(start, start);
        session
            .light
            .set_color_temperature(ColorTemperature::new(tint.kelvin())?)?;
        session.snapshot("stored_while_off")?;
        session.light.set_active(true)?;
        session.snapshot("switched_on")?;
        let outcome = if session.display_shows(tint, "switched_on")? {
            "applied"
        } else {
            "ignored"
        };
        session.record(
            "while_off_outcome",
            json!({"trial": trial, "outcome": outcome}),
        )?;
        outcomes.push(outcome);
    }
    say(&format!(
        "Stored while off, then switched on: {}",
        outcomes.join(", ")
    ))
}
