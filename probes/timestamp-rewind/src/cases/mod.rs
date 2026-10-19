//! One function per probe command. Each mutating case records its readings
//! and observer answers in `results/`, and `Session::run` restores Night
//! Light when it ends.

pub(crate) mod blocks;
pub(crate) mod display;
pub(crate) mod revert;

use crate::observer::Observer;
use crate::session::Session;
use crate::session::Tint;
use crate::session::say;
use crate::shell::ShellProcess;
use crate::snapshot::Snapshot;
use serde_json::json;
use std::time::Duration;
use std::time::Instant;
use win_nightlight_lib::Nightlight;

const WATCH_POLL: Duration = Duration::from_millis(5);
const SHELL_SETTLE: Duration = Duration::from_secs(3);

/// Print each change to the settings or their `Cloud` record for `seconds`
/// without writing anything.
pub(crate) fn watch(observer: &Observer, seconds: u64) -> anyhow::Result<()> {
    let light = Nightlight::new();
    let started = Instant::now();
    let mut previous: Option<Snapshot> = None;
    say("Type quit and press Enter to stop early.")?;
    while started.elapsed() < Duration::from_secs(seconds) {
        observer.check_quit()?;
        let snapshot = Snapshot::read(&light)?;
        if previous
            .as_ref()
            .is_none_or(|before| !before.same_values(&snapshot))
        {
            say(&snapshot.to_json_since(started)?.to_string())?;
            previous = Some(snapshot);
        }
        std::thread::sleep(WATCH_POLL);
    }
    Ok(())
}

/// Restart `ShellHost.exe`, then refresh and revert a write below `Cloud`
/// before and after Quick Settings opens, so a trace shows when
/// `ShellHost.exe` starts reading and reverting the settings.
pub(crate) fn quick_settings_reader(session: &mut Session) -> anyhow::Result<()> {
    restart_shell(session, ShellProcess::ShellHost)?;
    session.refresh(Tint::Neutral, "refresh_before_quick_settings")?;
    display::revert_below_cloud(session)?;
    // Quick Settings closes when the terminal takes focus, so it cannot stay
    // open while the probe writes.
    session.wait_for_observer("Open Quick Settings with Windows+A, then close it.")?;
    session.refresh(Tint::Yellow, "refresh_after_quick_settings")?;
    display::revert_below_cloud(session)?;
    Ok(())
}

/// Restart `process`, wait for the shell to settle, and record the new
/// process IDs.
fn restart_shell(session: &mut Session, process: ShellProcess) -> anyhow::Result<()> {
    let ids = process.restart()?;
    std::thread::sleep(SHELL_SETTLE);
    session.record("restarted", json!({"process": process.name(), "ids": ids}))
}

fn applied_or_ignored(applied: bool) -> &'static str {
    if applied { "applied" } else { "ignored" }
}

/// Return a tint other than `shown`, so the observer can see it apply, and
/// other than `stored`, so a preview to it changes the temperature.
fn contrasting(shown: Tint, stored: Tint) -> Tint {
    [Tint::Neutral, Tint::Yellow, Tint::Red]
        .into_iter()
        .find(|tint| *tint != shown && *tint != stored)
        .unwrap_or(Tint::Neutral)
}
