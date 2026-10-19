//! Cases that change the temperature with preview held on, as dragging the
//! Night Light slider in Windows Settings does, and ask how each fade looks.

use super::applied_or_ignored;
use super::restart_shell;
use crate::registry;
use crate::session::Session;
use crate::session::Smoothness;
use crate::session::Tint;
use crate::session::say;
use crate::shell;
use crate::shell::ShellProcess;
use crate::snapshot::modified_secs;
use anyhow::Context as _;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use win_nightlight_lib::ColorTemperature;
use win_nightlight_lib::Nightlight;

const SHORT_DRAG: Duration = Duration::from_secs(1);
const LONG_DRAG: Duration = Duration::from_secs(30);
const FADE_STEPS: u32 = 20;
const STEP_INTERVAL: Duration = Duration::from_secs(3);
const SWEEP_RATES_HZ: [u32; 4] = [30, 60, 144, 240];
const INTERFERENCE_DRAG: Duration = Duration::from_secs(30);
const INTERFERENCE_RATE_HZ: u32 = 30;
const BACK_AND_FORTH_PERIOD_SECS: u32 = 10;
const RESTART_DRAG: Duration = Duration::from_secs(30);
const RESTART_RATE_HZ: u32 = 240;
// The fade stays near neutral, so an answer of red or yellow comes from the
// writes after the release.
const NEAR_NEUTRAL_KELVIN: u16 = 5500;
const HOLD: Duration = Duration::from_secs(120);
const COUNTDOWN: Duration = Duration::from_secs(3);
const WRITE_LEAD_SECS: u64 = 2;

/// A slider-drag pattern that fades the temperature from neutral to red.
#[derive(Debug, Clone, Copy)]
enum Fade {
    /// Preview stays on for the whole fade, with `rate_hz` writes per second.
    Held { duration: Duration, rate_hz: u32 },
    /// [`FADE_STEPS`] steps [`STEP_INTERVAL`] apart, each turning preview on
    /// with the new temperature and off again [`registry::PREVIEW_STEP`]
    /// later.
    PerStep,
}

impl Fade {
    fn writes(self) -> anyhow::Result<u64> {
        Ok(match self {
            Self::Held { duration, rate_hz } => u64::from(held_steps(duration, rate_hz)?) + 2,
            Self::PerStep => u64::from(FADE_STEPS) * 2,
        })
    }

    fn to_json(self) -> anyhow::Result<Value> {
        Ok(match self {
            Self::Held { duration, rate_hz } => json!({
                "pattern": "held",
                "duration_ms": u64::try_from(duration.as_millis())?,
                "rate_hz": rate_hz,
            }),
            Self::PerStep => json!({
                "pattern": "per_step",
                "steps": FADE_STEPS,
                "interval_ms": u64::try_from(STEP_INTERVAL.as_millis())?,
            }),
        })
    }
}

/// When each step of a held drag finished, and the steps that found preview
/// off before writing.
#[derive(Default)]
struct DragLog {
    finished: Vec<Duration>,
    preview_was_off: Vec<usize>,
}

impl DragLog {
    fn push(&mut self, finished: Duration, preview_was_on: bool) {
        if !preview_was_on {
            self.preview_was_off.push(self.finished.len() + 1);
        }
        self.finished.push(finished);
    }

    fn to_json(&self) -> anyhow::Result<Value> {
        let mut gaps = self
            .finished
            .iter()
            .zip(self.finished.iter().skip(1))
            .map(|(earlier, later)| u64::try_from(later.saturating_sub(*earlier).as_micros()))
            .collect::<Result<Vec<_>, _>>()?;
        gaps.sort_unstable();
        let percentile = |percent: usize| {
            gaps.get(gaps.len().saturating_sub(1) * percent / 100)
                .copied()
        };
        Ok(json!({
            "steps": self.finished.len(),
            "last_step_us": self
                .finished
                .last()
                .map(|last| u64::try_from(last.as_micros()))
                .transpose()?,
            "median_gap_us": percentile(50),
            "p99_gap_us": percentile(99),
            "max_gap_us": gaps.last(),
            "steps_with_preview_off": self.preview_was_off,
        }))
    }
}

fn held_steps(duration: Duration, rate_hz: u32) -> anyhow::Result<u32> {
    let steps = duration.as_millis() * u128::from(rate_hz) / 1000;
    Ok(u32::try_from(steps).context("too many drag steps")?.max(1))
}

/// Return the temperature `index` steps of `steps` along a straight line from
/// `from` to `to` kelvin, rounded half up to a multiple of 10 K.
fn interpolate(from: u16, to: u16, index: u32, steps: u32) -> anyhow::Result<ColorTemperature> {
    let from = i64::from(from);
    let to = i64::from(to);
    let steps = i64::from(steps);
    let scaled = from * steps + (to - from) * i64::from(index);
    let tens = (2 * scaled + 10 * steps).div_euclid(20 * steps);
    Ok(ColorTemperature::new(u16::try_from(tens * 10)?)?)
}

/// Return the temperature at step `index` of a fade at `rate_hz` steps per
/// second that moves from `from` to `to` kelvin and back every
/// [`BACK_AND_FORTH_PERIOD_SECS`].
fn back_and_forth(
    from: u16,
    to: u16,
    index: u32,
    rate_hz: u32,
) -> anyhow::Result<ColorTemperature> {
    let half_period = rate_hz * BACK_AND_FORTH_PERIOD_SECS / 2;
    let position = index % (2 * half_period);
    interpolate(
        from,
        to,
        half_period - position.abs_diff(half_period),
        half_period,
    )
}

fn neutral_to_red(index: u32, steps: u32) -> anyhow::Result<ColorTemperature> {
    interpolate(Tint::Neutral.kelvin(), Tint::Red.kelvin(), index, steps)
}

pub(super) fn sleep_until(deadline: Instant) {
    if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
        std::thread::sleep(wait);
    }
}

pub(super) fn count_down(text: &str) -> anyhow::Result<()> {
    say(&format!(
        "Watch the screen: {text} It starts in {} s.",
        COUNTDOWN.as_secs()
    ))?;
    std::thread::sleep(COUNTDOWN);
    Ok(())
}

/// Print how far `writes` settings writes can move the settings timestamp
/// ahead of real time, and wait for the observer to continue.
fn confirm_lead(session: &mut Session, writes: u64) -> anyhow::Result<()> {
    let lead_secs = writes * WRITE_LEAD_SECS;
    session.record(
        "projected_lead",
        json!({"writes": writes, "lead_s": lead_secs}),
    )?;
    session.confirm(&format!(
        "This case makes about {writes} settings writes, which can move the settings timestamp \
         {} h {} min ahead of real time. Night Light keeps working, and the lead shrinks only as \
         real time passes.",
        lead_secs / 3600,
        lead_secs % 3600 / 60
    ))
}

/// Return the refresh rate of the fastest active display, rounded to whole
/// hertz.
fn fastest_display_hz() -> anyhow::Result<u32> {
    let environment: Value = serde_json::from_str(&shell::powershell_script("environment.ps1")?)?;
    environment
        .get("active_display_refresh_rates_hz")
        .and_then(Value::as_array)
        .context("the environment script did not list display refresh rates")?
        .iter()
        .filter_map(Value::as_str)
        .filter_map(parse_refresh_rate)
        .max()
        .context("no active display reported a refresh rate")
}

/// Parse the first nonzero rate in a display refresh-rate entry, such as
/// `479989/1000 path, 479989/1000 signal`, rounded half up to whole hertz.
fn parse_refresh_rate(entry: &str) -> Option<u32> {
    entry
        .split(',')
        .filter_map(|part| {
            let (numerator, denominator) = part.split_whitespace().next()?.split_once('/')?;
            let numerator: u64 = numerator.parse().ok()?;
            let denominator: u64 = denominator.parse().ok().filter(|value| *value > 0)?;
            u32::try_from((2 * numerator + denominator) / (2 * denominator)).ok()
        })
        .find(|hz| *hz > 0)
}

/// Turn preview on, write a temperature from `temperature_at(index, steps)`
/// at each of `rate_hz` steps per second over `duration`, and turn preview
/// off.
fn held_drag(
    session: &mut Session,
    duration: Duration,
    rate_hz: u32,
    temperature_at: impl Fn(u32, u32) -> anyhow::Result<ColorTemperature>,
) -> anyhow::Result<()> {
    let steps = held_steps(duration, rate_hz)?;
    let mut log = DragLog::default();
    let start_found_preview = session.light.start_preview()?;
    let started = Instant::now();
    for index in 1..=steps {
        session.check_quit()?;
        sleep_until(started + duration * index / steps);
        let preview_was_on = session
            .light
            .preview_color_temperature(temperature_at(index, steps)?)?;
        log.push(started.elapsed(), preview_was_on);
    }
    let end_found_preview = session.light.end_preview()?;
    session.record(
        "held_drag",
        json!({
            "rate_hz": rate_hz,
            "duration_ms": u64::try_from(duration.as_millis())?,
            "start_found_preview": start_found_preview,
            "end_found_preview": end_found_preview,
            "log": log.to_json()?,
        }),
    )
}

/// Write `temperature` with preview on and turn preview off
/// [`registry::PREVIEW_STEP`] later, as a click on the slider does.
fn step_drag(session: &mut Session, temperature: ColorTemperature) -> anyhow::Result<Value> {
    let preview_was_on = session.light.preview_color_temperature(temperature)?;
    let turned_on = Instant::now();
    std::thread::sleep(registry::PREVIEW_STEP);
    let preview_was_on_at_end = session.light.end_preview()?;
    Ok(json!({
        "kelvin": temperature.kelvin(),
        "preview_was_on": preview_was_on,
        "preview_was_on_at_end": preview_was_on_at_end,
        "held_us": u64::try_from(turned_on.elapsed().as_micros())?,
    }))
}

/// Fade from neutral to red in [`FADE_STEPS`] steps [`STEP_INTERVAL`] apart,
/// making each step with `write`, and record what each step returned.
pub(super) fn stepped_fade(
    session: &mut Session,
    event: &str,
    mut write: impl FnMut(&mut Session, ColorTemperature) -> anyhow::Result<Value>,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let mut steps = Vec::new();
    for index in 1..=FADE_STEPS {
        session.check_quit()?;
        sleep_until(started + STEP_INTERVAL * (index - 1));
        let offset = u64::try_from(started.elapsed().as_millis())?;
        let result = write(session, neutral_to_red(index, FADE_STEPS)?)?;
        steps.push(json!({"offset_ms": offset, "result": result}));
    }
    session.record(event, json!({"steps": steps}))
}

/// Ask how the fade looked and whether it ended on red, and record both
/// answers with `details`.
pub(super) fn ask_fade_outcome(
    session: &mut Session,
    event: &str,
    details: &Value,
) -> anyhow::Result<(Option<Smoothness>, bool)> {
    session.snapshot("fade_end")?;
    let looked = session.fade_looked("fade")?;
    let ended_red = session.display_shows(Tint::Red, "fade_end")?;
    session.record(
        event,
        json!({
            "details": details,
            "looked": looked.map_or("other", Smoothness::name),
            "ended_red": ended_red,
        }),
    )?;
    Ok((looked, ended_red))
}

/// Fade from neutral to red with `fade`, after an `explorer.exe` restart when
/// `after_restart` is set, and ask how the fade looked.
fn fade_trial(session: &mut Session, fade: Fade, after_restart: bool) -> anyhow::Result<()> {
    session.refresh(Tint::Neutral, "fade_setup")?;
    if after_restart {
        // The new explorer.exe applies the stored neutral temperature at
        // startup and then ignores ordinary refreshes.
        restart_shell(session, ShellProcess::Explorer)?;
    }
    let started_neutral = session.display_shows(Tint::Neutral, "fade_start")?;
    match fade {
        Fade::Held { duration, rate_hz } => {
            count_down("the temperature fades from neutral to red.")?;
            held_drag(session, duration, rate_hz, neutral_to_red)?;
        }
        Fade::PerStep => {
            count_down(&format!(
                "the temperature changes from neutral to red in {FADE_STEPS} steps, {} s apart.",
                STEP_INTERVAL.as_secs()
            ))?;
            stepped_fade(session, "stepped_fade", step_drag)?;
        }
    }
    let (looked, ended_red) = ask_fade_outcome(
        session,
        "fade_outcome",
        &json!({
            "fade": fade.to_json()?,
            "after_restart": after_restart,
            "started_neutral": started_neutral,
        }),
    )?;
    say(&format!(
        "{fade:?}{}: looked {}, ended on red: {ended_red}",
        if after_restart {
            " after an explorer.exe restart"
        } else {
            ""
        },
        looked.map_or("other", Smoothness::name)
    ))
}

/// Fade from neutral to red with held drags of 1 s and 30 s at `rate_hz`, and
/// with per-step drags 3 s apart, first as is and then after an
/// `explorer.exe` restart before each fade.
pub(crate) fn drag_fade(session: &mut Session, rate_hz: Option<u32>) -> anyhow::Result<()> {
    let rate_hz = match rate_hz {
        Some(rate_hz) => rate_hz,
        None => fastest_display_hz()?,
    };
    let fades = [
        Fade::Held {
            duration: SHORT_DRAG,
            rate_hz,
        },
        Fade::Held {
            duration: LONG_DRAG,
            rate_hz,
        },
        Fade::PerStep,
    ];
    let mut writes = 0;
    for fade in fades {
        writes += 2 * fade.writes()?;
    }
    confirm_lead(session, writes)?;
    session.require_working_display()?;
    for after_restart in [false, true] {
        for fade in fades {
            fade_trial(session, fade, after_restart)?;
        }
    }
    Ok(())
}

fn clock_seed() -> anyhow::Result<u64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    Ok(now.as_secs().wrapping_mul(1_000_000_007) ^ u64::from(now.subsec_nanos()) | 1)
}

/// Shuffle `items` in place with a xorshift generator whose state is `state`,
/// which must not be zero.
fn shuffle<T>(items: &mut [T], state: &mut u64) -> anyhow::Result<()> {
    for last in (1..items.len()).rev() {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        let pick = usize::try_from(*state % u64::try_from(last + 1)?)?;
        items.swap(last, pick);
    }
    Ok(())
}

/// Make 1-second held drags from neutral to red at several write rates, up to
/// the fastest display's refresh rate, in a new random order each round, and
/// ask how each fade looked without naming its rate.
pub(crate) fn drag_rate(session: &mut Session, rounds: u32) -> anyhow::Result<()> {
    let fastest = fastest_display_hz()?;
    let mut rates: Vec<u32> = SWEEP_RATES_HZ
        .into_iter()
        .filter(|rate_hz| *rate_hz < fastest)
        .collect();
    rates.push(fastest);
    let mut writes = 0;
    for rate_hz in &rates {
        writes += u64::from(rounds)
            * Fade::Held {
                duration: SHORT_DRAG,
                rate_hz: *rate_hz,
            }
            .writes()?;
    }
    confirm_lead(session, writes)?;
    session.require_working_display()?;
    let mut state = clock_seed()?;
    session.record("shuffle_seed", json!({"seed": state}))?;
    let mut answers: Vec<(u32, &str, bool)> = Vec::new();
    for round in 1..=rounds {
        let mut order = rates.clone();
        shuffle(&mut order, &mut state)?;
        for (trial, rate_hz) in order.into_iter().enumerate() {
            say(&format!(
                "Round {round} of {rounds}, fade {} of {}.",
                trial + 1,
                rates.len()
            ))?;
            session.refresh(Tint::Neutral, "fade_setup")?;
            count_down("the temperature fades from neutral to red in 1 s.")?;
            held_drag(session, SHORT_DRAG, rate_hz, neutral_to_red)?;
            let (looked, ended_red) = ask_fade_outcome(
                session,
                "rate_outcome",
                &json!({"round": round, "trial": trial + 1, "rate_hz": rate_hz}),
            )?;
            answers.push((rate_hz, looked.map_or("other", Smoothness::name), ended_red));
        }
    }
    answers.sort_by_key(|(rate_hz, _, _)| *rate_hz);
    let summary: Vec<String> = answers
        .iter()
        .map(|(rate_hz, looked, ended_red)| {
            format!("{rate_hz} Hz: looked {looked}, ended on red: {ended_red}")
        })
        .collect();
    say(&format!("Rate sweep results:\n{}", summary.join("\n")))
}

/// Hold a slider drag that fades between neutral and red for 30 s at
/// `rate_hz`, 30 by default, while the observer drags the Windows Settings
/// slider, then restore yellow with a per-step drag, as an application that
/// manages the temperature would.
pub(crate) fn drag_interference(session: &mut Session, rate_hz: Option<u32>) -> anyhow::Result<()> {
    let rate_hz = rate_hz.unwrap_or(INTERFERENCE_RATE_HZ);
    let drag = Fade::Held {
        duration: INTERFERENCE_DRAG,
        rate_hz,
    };
    confirm_lead(session, drag.writes()? + 2)?;
    session.require_working_display()?;
    session
        .wait_for_observer("Open Settings > System > Display > Night light and keep it open.")?;
    say(&format!(
        "For {} s the probe fades between neutral and red with preview held on, as a slider \
         drag does. While it runs, press the Strength slider, drag it, and release it once, \
         before the probe's drag ends. Then leave the slider alone until the probe asks a \
         question.",
        INTERFERENCE_DRAG.as_secs()
    ))?;
    count_down("the probe's drag begins.")?;
    held_drag(session, INTERFERENCE_DRAG, rate_hz, |index, _steps| {
        back_and_forth(Tint::Neutral.kelvin(), Tint::Red.kelvin(), index, rate_hz)
    })?;
    session.snapshot("after_interference")?;
    let restore = step_drag(session, ColorTemperature::new(Tint::Yellow.kelvin())?)?;
    session.record("restore_after_interference", restore)?;
    let restored = session.display_shows(Tint::Yellow, "restored_after_interference")?;
    session.wait_for_observer("Close Settings.")?;
    say(&format!(
        "Per-step drag to yellow after the interference: {}",
        applied_or_ignored(restored)
    ))
}

/// Hold a slider drag at `rate_hz`, 240 by default, that fades near neutral
/// while the observer drags and releases the Windows Settings slider. Stop
/// writing at the release, as an application that exits would. Then make a
/// per-step drag to red through a new handle and one to yellow through a new
/// handle with the startup margin, and ask after each which tint shows.
pub(crate) fn restart_after_slider(
    session: &mut Session,
    rate_hz: Option<u32>,
) -> anyhow::Result<()> {
    let rate_hz = rate_hz.unwrap_or(RESTART_RATE_HZ);
    let steps = held_steps(RESTART_DRAG, rate_hz)?;
    confirm_lead(session, u64::from(steps) + 6)?;
    session.require_working_display()?;
    session
        .wait_for_observer("Open Settings > System > Display > Night light and keep it open.")?;
    say(&format!(
        "For up to {} s the probe fades near neutral with preview held on, as a slider drag \
         does. While it runs, press the Strength slider, drag it for about 3 s, and release it \
         near its left end within 20 s. The probe stops writing when you release the slider. \
         Then leave the slider alone until the probe asks a question.",
        RESTART_DRAG.as_secs()
    ))?;
    count_down("the probe's drag begins.")?;
    let (drag, gap_secs) = drag_until_release(session, rate_hz, steps)?;
    session.record("held_drag", drag)?;
    session.snapshot("after_release")?;
    // A new handle's per-step drag stamps its two writes 2 s and 4 s above the
    // stored timestamp. With a smaller gap, explorer.exe would apply the second
    // write without the margin.
    let Some(gap_secs) = gap_secs.filter(|gap| *gap >= 2 * WRITE_LEAD_SECS) else {
        session.wait_for_observer("Close Settings.")?;
        return say(&format!(
            "The drag did not stop at a release stamped at least {} s below the probe's highest \
             write, so the probe skipped the writes through new handles.",
            2 * WRITE_LEAD_SECS
        ));
    };
    session.light = Nightlight::new();
    let control = step_drag(session, ColorTemperature::new(Tint::Red.kelvin())?)?;
    session.record("new_handle", control)?;
    session.snapshot("after_new_handle")?;
    let control_applied = session.display_shows(Tint::Red, "new_handle")?;
    session.light = Nightlight::new().with_startup_margin();
    let margin = step_drag(session, ColorTemperature::new(Tint::Yellow.kelvin())?)?;
    session.record("new_handle_with_margin", margin)?;
    session.snapshot("after_new_handle_with_margin")?;
    let margin_applied = session.display_shows(Tint::Yellow, "new_handle_with_margin")?;
    session.wait_for_observer("Close Settings.")?;
    say(&format!(
        "The release left the stored timestamp {gap_secs} s below the probe's highest write. \
         Per-step drag through a new handle: {}. Through a new handle with the startup margin: \
         {}.",
        applied_or_ignored(control_applied),
        applied_or_ignored(margin_applied)
    ))
}

/// How a drag that waits for the observer's release ended.
enum DragEnd {
    /// A read before step `step` found preview off and the settings
    /// timestamp `stored`.
    Released {
        step: u32,
        stored: u64,
    },
    /// The probe's write at step `step` found preview off and turned it back
    /// on.
    WriteFoundPreviewOff {
        step: u32,
    },
    Completed,
}

/// Make a held drag of `steps` steps at `rate_hz` that fades between neutral
/// and [`NEAR_NEUTRAL_KELVIN`], and stop without writing again as soon as a
/// read finds preview off. Return the drag's record and, when the drag
/// stopped that way, how many seconds the stored timestamp was below the
/// probe's highest write.
fn drag_until_release(
    session: &mut Session,
    rate_hz: u32,
    steps: u32,
) -> anyhow::Result<(Value, Option<u64>)> {
    let mut log = DragLog::default();
    let start_found_preview = session.light.start_preview()?;
    let mut highest_written = modified_secs(&session.light.read_settings()?)?;
    let mut end = DragEnd::Completed;
    let started = Instant::now();
    for index in 1..=steps {
        session.check_quit()?;
        sleep_until(started + RESTART_DRAG * index / steps);
        let stored = session.light.read_settings()?;
        if !stored.is_previewing() {
            end = DragEnd::Released {
                step: index,
                stored: modified_secs(&stored)?,
            };
            break;
        }
        let temperature =
            back_and_forth(Tint::Neutral.kelvin(), NEAR_NEUTRAL_KELVIN, index, rate_hz)?;
        let preview_was_on = session.light.preview_color_temperature(temperature)?;
        log.push(started.elapsed(), preview_was_on);
        highest_written = highest_written.max(modified_secs(&session.light.read_settings()?)?);
        // A release between the probe's read and the library's read inside
        // this write ends the drag here. A release between the library's read
        // and its write is turned back on without a sign in the return value.
        if !preview_was_on {
            end = DragEnd::WriteFoundPreviewOff { step: index };
            break;
        }
    }
    if matches!(end, DragEnd::Completed) {
        session.light.end_preview()?;
    }
    session.note_settings_modified(highest_written);
    let (end_name, end_step, stored, gap_secs) = match end {
        DragEnd::Released { step, stored } => (
            "released",
            Some(step),
            Some(stored),
            Some(highest_written.saturating_sub(stored)),
        ),
        DragEnd::WriteFoundPreviewOff { step } => {
            ("write_found_preview_off", Some(step), None, None)
        }
        DragEnd::Completed => ("completed", None, None, None),
    };
    let record = json!({
        "rate_hz": rate_hz,
        "start_found_preview": start_found_preview,
        "end": end_name,
        "end_step": end_step,
        "stored_at_release_unix_s": stored,
        "highest_written_unix_s": highest_written,
        "gap_s": gap_secs,
        "log": log.to_json()?,
    });
    Ok((record, gap_secs))
}

/// Write `tint` while preview stays on, ask whether it applied, and record
/// whether preview and Night Light were on before the write.
fn hold_step(session: &mut Session, tint: Tint, step: &str) -> anyhow::Result<Value> {
    let before = session.snapshot(&format!("{step}_before"))?;
    let active = session.light.read_state()?.is_active();
    let preview_was_on = session
        .light
        .preview_color_temperature(ColorTemperature::new(tint.kelvin())?)?;
    let applied = session.display_shows(tint, step)?;
    say(&format!(
        "{step}: preview was {}, Night Light was {}, the {tint:?} write {}",
        if preview_was_on { "on" } else { "off" },
        if active { "on" } else { "off" },
        applied_or_ignored(applied)
    ))?;
    Ok(json!({
        "step": step,
        "previewing_before": before.previewing,
        "active": active,
        "preview_was_on": preview_was_on,
        "applied": applied,
    }))
}

/// Keep preview on for at least two minutes while the observer switches Night
/// Light off and opens Windows Settings, and while `explorer.exe` restarts.
/// After each event, check whether a write with preview on still applies.
pub(crate) fn drag_hold(session: &mut Session) -> anyhow::Result<()> {
    session.require_working_display()?;
    let started = Instant::now();
    let start_found_preview = session.light.start_preview()?;
    let mut steps = vec![hold_step(session, Tint::Yellow, "hold_start")?];
    session.wait_for_observer(
        "Open Quick Settings with Windows+A, switch Night light off, and close Quick Settings.",
    )?;
    steps.push(hold_step(session, Tint::Red, "hold_after_switching_off")?);
    session.wait_for_observer(
        "Open Settings > System > Display > Night light without touching any control, and keep \
         it open.",
    )?;
    steps.push(hold_step(
        session,
        Tint::Yellow,
        "hold_after_opening_settings",
    )?);
    restart_shell(session, ShellProcess::Explorer)?;
    steps.push(hold_step(
        session,
        Tint::Red,
        "hold_after_explorer_restart",
    )?);
    if let Some(remaining) = HOLD.checked_sub(started.elapsed()) {
        say(&format!(
            "Holding preview on for {} s more.",
            remaining.as_secs()
        ))?;
        sleep_until(Instant::now() + remaining);
    }
    steps.push(hold_step(session, Tint::Yellow, "hold_after_two_minutes")?);
    let end_found_preview = session.light.end_preview()?;
    let active = session.light.read_state()?.is_active();
    session.snapshot("hold_end")?;
    // With Night Light off, the screen shows no tint once preview ends.
    let expected = if active { Tint::Yellow } else { Tint::Neutral };
    let ended = session.display_shows(expected, "hold_end")?;
    session.record(
        "hold_outcome",
        json!({
            "held_s": started.elapsed().as_secs(),
            "start_found_preview": start_found_preview,
            "steps": steps,
            "end_found_preview": end_found_preview,
            "active_at_end": active,
            "end_applied": ended,
        }),
    )?;
    session.wait_for_observer("Close Settings.")?;
    say(&format!(
        "After preview ended, the screen showed the expected {expected:?}: {ended}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(0, 6500; "start")]
    #[test_case(1, 5180; "rounds 5175 up")]
    #[test_case(2, 3850; "middle")]
    #[test_case(3, 2530; "rounds 2525 up")]
    #[test_case(4, 1200; "end")]
    fn interpolates_in_10_kelvin_steps(index: u32, kelvin: u16) {
        let temperature = interpolate(Tint::Neutral.kelvin(), Tint::Red.kelvin(), index, 4)
            .expect("temperature is in range");
        assert_eq!(temperature.kelvin(), kelvin);
    }

    #[test]
    fn interpolates_toward_a_warmer_start() {
        let temperature = interpolate(Tint::Red.kelvin(), Tint::Yellow.kelvin(), 1, 2)
            .expect("temperature is in range");
        assert_eq!(temperature.kelvin(), 2300);
    }

    #[test_case("479989/1000 path, 479989/1000 signal", Some(480); "fractional rate")]
    #[test_case("60/1 path", Some(60); "whole rate")]
    #[test_case("121/2", Some(61); "rounds half up")]
    #[test_case("0/0 path, 60/1 signal", Some(60); "signal rate when path rate is zero")]
    #[test_case("0/0 path", None; "zero denominator")]
    #[test_case("unknown", None; "no rational")]
    fn parses_refresh_rate(entry: &str, expected: Option<u32>) {
        assert_eq!(parse_refresh_rate(entry), expected);
    }

    #[test_case(Duration::from_secs(1), 480, 480; "one second")]
    #[test_case(Duration::from_secs(30), 144, 4320; "thirty seconds")]
    #[test_case(Duration::from_millis(10), 30, 1; "at least one step")]
    fn counts_held_drag_steps(duration: Duration, rate_hz: u32, expected: u32) {
        assert_eq!(
            held_steps(duration, rate_hz).expect("step count fits"),
            expected
        );
    }

    #[test]
    fn shuffle_keeps_every_item() {
        let mut items = [30, 60, 144, 240, 480];
        let mut state = 0x9E37_79B9_7F4A_7C15;
        shuffle(&mut items, &mut state).expect("shuffle succeeds");
        let mut sorted = items;
        sorted.sort_unstable();
        assert_eq!(sorted, [30, 60, 144, 240, 480]);
        assert_ne!(state, 0x9E37_79B9_7F4A_7C15);
    }
}
