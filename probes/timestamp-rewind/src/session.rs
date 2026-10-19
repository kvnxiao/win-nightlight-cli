//! An interactive case run: preconditions, observer prompts, the results
//! file, and restoring Night Light afterwards.

use crate::observer::Observer;
use crate::registry;
use crate::shell;
use crate::shell::ShellProcess;
use crate::snapshot::Snapshot;
use crate::snapshot::unix_secs;
use crate::snapshot::utc;
use anyhow::Context as _;
use anyhow::ensure;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use serde_json::Value;
use serde_json::json;
use std::io::Write as _;
use std::time::SystemTime;
use win_nightlight_lib::ColorTemperature;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::RefreshPolicy;

/// A temperature chosen so an observer can tell it apart from the others at
/// a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tint {
    Neutral,
    Yellow,
    Red,
}

impl Tint {
    pub(crate) fn kelvin(self) -> u16 {
        match self {
            Self::Neutral => 6500,
            Self::Yellow => 3400,
            Self::Red => 1200,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Yellow => "yellow",
            Self::Red => "red",
        }
    }
}

// The question lists every tint and does not name the expected one, so an
// answer cannot confirm an expectation by habit. Answers are digits, not y or
// n, so an observer used to yes/no questions does not pick a tint by reflex.
const TINT_QUESTION: &str = "What does the screen show now? 1 neutral, no warm tint (6500 K) / 2 \
                             yellow (3400 K) / 3 deep red (1200 K) / 0 something else or unsure \
                             / quit";

pub(crate) struct Session {
    pub(crate) light: Nightlight,
    results: fs_err::File,
    path: Utf8PathBuf,
    observer: Observer,
    original_kelvin: Option<i16>,
    originally_active: bool,
    highest_modified: u64,
}

impl Session {
    /// Start a session for `case`, run `body`, and restore Night Light whether
    /// `body` succeeds, fails, or the observer stops it with `quit`.
    ///
    /// # Errors
    ///
    /// Returns the error of [`start`](Self::start) or `body`, and adds the
    /// restore error when restoring fails too.
    pub(crate) fn run(
        case: &str,
        observer: Observer,
        body: impl FnOnce(&mut Self) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut session = Self::start(case, observer)?;
        let outcome = body(&mut session);
        let noted = match &outcome {
            Ok(()) => Ok(()),
            Err(error) => session.record("stopped", json!({"error": format!("{error:#}")})),
        };
        let restored = noted.and_then(|()| session.restore());
        match (outcome, restored) {
            (Ok(()), restored) => restored,
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(restore_error)) => Err(error.context(format!(
                "restoring Night Light also failed: {restore_error:#}"
            ))),
        }
    }

    fn start(case: &str, observer: Observer) -> anyhow::Result<Self> {
        let light = Nightlight::new();
        let settings = light.read_settings()?;
        // Preview and an open Windows Settings window both write the settings
        // during a case.
        ensure!(
            !settings.is_previewing(),
            "Night Light preview is active; refusing to run"
        );
        ensure!(
            shell::powershell_script("settings-windows.ps1")? == "0",
            "a Windows Settings window is open; close it before running the probe"
        );
        let directory = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("results");
        fs_err::create_dir_all(&directory)?;
        let path = directory.join(format!("{case}-{}.jsonl", unix_secs(SystemTime::now())?));
        let mut session = Self {
            results: fs_err::File::create_new(&path)?,
            path,
            observer,
            original_kelvin: settings.color_temperature(),
            originally_active: light.read_state()?.is_active(),
            highest_modified: 0,
            light,
        };
        say(&format!("Writing results to {}", session.path))?;
        say("Type quit and press Enter to stop early; the probe then restores Night Light.")?;
        session.record(
            "environment",
            json!({
                "case": case,
                "environment": serde_json::from_str::<Value>(&shell::powershell_script("environment.ps1")?)?,
                "git_revision": shell::capture("git", &["rev-parse", "HEAD"])?,
                "git_status": shell::capture("git", &["status", "--porcelain", "--", ":/probes", ":(exclude)probes/timestamp-rewind/results"])?,
                "rustc": shell::capture("rustc", &["--version"])?,
            }),
        )?;
        session.light.set_active(true)?;
        session.snapshot("start")?;
        Ok(session)
    }

    pub(crate) fn record(&mut self, event: &str, data: Value) -> anyhow::Result<()> {
        let mut line = serde_json::Map::new();
        line.insert("event".to_owned(), event.into());
        line.insert("utc".to_owned(), utc(SystemTime::now())?.into());
        line.insert("data".to_owned(), data);
        writeln!(self.results, "{}", Value::Object(line)).context("writing results record")
    }

    /// Read and record the settings, tracking the highest timestamp seen.
    pub(crate) fn snapshot(&mut self, event: &str) -> anyhow::Result<Snapshot> {
        let snapshot = Snapshot::read(&self.light)?;
        self.highest_modified = self.highest_modified.max(snapshot.settings_modified);
        let value = json!({
            "snapshot": snapshot.to_json()?,
            "shell_process_ids": shell_process_ids()?,
        });
        self.record(event, value)?;
        Ok(snapshot)
    }

    pub(crate) fn highest_modified(&self) -> u64 {
        self.highest_modified
    }

    pub(crate) fn refresh(&mut self, tint: Tint, event: &str) -> anyhow::Result<Snapshot> {
        self.light
            .set_color_temperature(ColorTemperature::new(tint.kelvin())?, RefreshPolicy::Always)?;
        self.snapshot(event)
    }

    /// Change the temperature to `tint` inside a preview, which applies after
    /// an `explorer.exe` restart, and record the stored result.
    pub(crate) fn preview_refresh(&mut self, tint: Tint, event: &str) -> anyhow::Result<Snapshot> {
        registry::preview_refresh(&self.light, ColorTemperature::new(tint.kelvin())?)?;
        self.snapshot(event)
    }

    /// Run `count` complete refreshes to `tint` to advance the settings
    /// timestamp ahead of real time.
    pub(crate) fn burst(&mut self, tint: Tint, count: u32) -> anyhow::Result<Snapshot> {
        let temperature = ColorTemperature::new(tint.kelvin())?;
        for _ in 0..count {
            self.light
                .set_color_temperature(temperature, RefreshPolicy::Always)?;
        }
        self.snapshot("burst")
    }

    /// Ask which tint the display shows, record the answer with the
    /// expected `tint`, and return whether they match.
    pub(crate) fn display_shows(&mut self, tint: Tint, step: &str) -> anyhow::Result<bool> {
        let seen = self.ask_tint()?;
        self.record(
            "observer",
            json!({
                "step": step,
                "expected": tint.name(),
                "answer": seen.map_or("other", Tint::name),
            }),
        )?;
        Ok(seen == Some(tint))
    }

    /// Refresh to neutral and then to red, and ask after each, so a display
    /// that already showed one of them cannot pass the check.
    ///
    /// # Errors
    ///
    /// Returns an error if the observer does not see either change, because
    /// the case's later checks then cannot distinguish its effect.
    pub(crate) fn require_working_display(&mut self) -> anyhow::Result<()> {
        for tint in [Tint::Neutral, Tint::Red] {
            self.refresh(tint, "control")?;
            ensure!(
                self.display_shows(tint, "control")?,
                "the display does not follow refreshes before the case starts; move the \
                 Night Light Strength slider in Windows Settings, or sign out and in, then retry"
            );
        }
        Ok(())
    }

    /// Write the settings timestamp above every timestamp this case has seen,
    /// so later refreshes apply, then restore the original temperature inside
    /// a preview, which also applies after an `explorer.exe` restart, and the
    /// original on/off state.
    pub(crate) fn restore(&mut self) -> anyhow::Result<()> {
        let above_every_seen = self
            .highest_modified
            .max(registry::read_cloud_modified()?)
            .checked_add(1)
            .context("settings timestamp overflows")?;
        registry::write_settings_modified(above_every_seen)?;
        if let Some(kelvin) = self
            .original_kelvin
            .and_then(|kelvin| u16::try_from(kelvin).ok())
        {
            registry::preview_refresh(&self.light, ColorTemperature::new(kelvin)?)?;
        }
        self.light.set_active(self.originally_active)?;
        self.snapshot("restored")?;
        say(&format!(
            "Restored the original temperature and on/off state. Results: {}",
            self.path
        ))
    }

    pub(crate) fn check_quit(&self) -> anyhow::Result<()> {
        self.observer.check_quit()
    }

    pub(crate) fn wait_for_observer(&mut self, instruction: &str) -> anyhow::Result<()> {
        self.observer.check_quit()?;
        say(&format!("{instruction} Press Enter when done."))?;
        self.observer.next_line()?;
        self.record("observer_action", json!({"instruction": instruction}))
    }

    fn ask_tint(&mut self) -> anyhow::Result<Option<Tint>> {
        // A line typed before the question appears would answer it without
        // the observer looking at the screen.
        self.observer.check_quit()?;
        loop {
            say(TINT_QUESTION)?;
            match self.observer.next_line()?.trim() {
                "1" => return Ok(Some(Tint::Neutral)),
                "2" => return Ok(Some(Tint::Yellow)),
                "3" => return Ok(Some(Tint::Red)),
                "0" => return Ok(None),
                _ => {}
            }
        }
    }
}

fn shell_process_ids() -> anyhow::Result<Value> {
    Ok(json!({
        "explorer": ShellProcess::Explorer.ids()?,
        "shellhost": ShellProcess::ShellHost.ids()?,
    }))
}

pub(crate) fn say(text: &str) -> anyhow::Result<()> {
    let mut output = std::io::stdout().lock();
    writeln!(output, "{text}")?;
    output.flush().context("flushing probe output")
}
