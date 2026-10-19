//! Probe that measures how Windows handles writes that move the Night Light
//! settings timestamp backward.
//!
//! Every command except `watch` changes live Night Light settings, and the
//! display cases need an observer at the screen. See `README.md` for what each
//! case measures and how to run it.

mod cases;
mod envelope;
mod observer;
mod registry;
mod session;
mod shell;
mod snapshot;

use anyhow::Context as _;
use anyhow::ensure;
use cases::blocks::Blocker;
use clap::Parser;
use clap::Subcommand;
use observer::Observer;
use observer::Quit;
use session::Session;
use session::say;
use std::process::ExitCode;
use std::time::Duration;

const FREEZE_BURST: u32 = 256;
// The justfile recipes treat this exit code as success.
const QUIT_EXIT_CODE: u8 = 3;
const SUITE_PARTS: u8 = 4;
const SUITE_PAUSE: Duration = Duration::from_secs(2);
// Every case in the order the suite runs it, with its part. The order keeps
// cases from changing each other's conditions:
// - The `Cloud`-ahead replay runs first, while the stored timestamp is close to
//   real time, because it waits for real time to pass the stored timestamp.
// - Cases that write `Cloud`, or ask for a slider write, run last, because they
//   change the timestamp that writes below `Cloud` revert to.
const SUITE: [(u8, Case); 22] = [
    (
        1,
        Case::RevertsWhileOff {
            burst: FREEZE_BURST,
            cloud_ahead: true,
        },
    ),
    (1, Case::Revert),
    (1, Case::CloudBoundary),
    (1, Case::RevertWindow { passes: 15 }),
    (
        1,
        Case::RevertsWhileOff {
            burst: FREEZE_BURST,
            cloud_ahead: false,
        },
    ),
    (
        1,
        Case::RevertAfterFreeze {
            burst: FREEZE_BURST,
        },
    ),
    (
        2,
        Case::Freeze {
            burst: FREEZE_BURST,
        },
    ),
    (
        2,
        Case::HeldPreviewDuringFreeze {
            burst: FREEZE_BURST,
        },
    ),
    (
        2,
        Case::SwitchRelease {
            blocker: Blocker::Freeze,
        },
    ),
    (
        2,
        Case::PreviewTiming {
            blocker: Blocker::Freeze,
            rounds: 1,
            timing: None,
        },
    ),
    (
        2,
        Case::PreviewTiming {
            blocker: Blocker::Freeze,
            rounds: 4,
            timing: Some((0, 0)),
        },
    ),
    (
        3,
        Case::PreviewTiming {
            blocker: Blocker::None,
            rounds: 1,
            timing: None,
        },
    ),
    (3, Case::QuickSettingsReader),
    (3, Case::RestartShellhost),
    (3, Case::FreezeSurvivesRestart),
    (
        3,
        Case::SwitchRelease {
            blocker: Blocker::ExplorerRestart,
        },
    ),
    (
        3,
        Case::PreviewTiming {
            blocker: Blocker::ExplorerRestart,
            rounds: 1,
            timing: None,
        },
    ),
    (
        4,
        Case::ExplorerRestartAfterFreeze {
            burst: FREEZE_BURST,
        },
    ),
    (4, Case::HeldPreviewAfterExplorerRestart),
    (4, Case::LowerCloud),
    (4, Case::ResetBoth),
    (4, Case::RestartExplorer),
];

#[derive(Debug, Parser)]
#[command(about, version)]
struct Cli {
    #[command(subcommand)]
    case: Case,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum Case {
    /// Print each change to the settings and their Cloud timestamp without
    /// writing anything.
    Watch {
        /// How long to watch.
        #[arg(default_value_t = 30)]
        seconds: u64,
    },
    /// Write the settings below the Cloud timestamp and record how Windows
    /// replaces them.
    Revert,
    /// Write the settings at the Cloud timestamp and one second below it.
    CloudBoundary,
    /// Refresh at growing delays after a reverted write and count overwritten
    /// refreshes.
    RevertWindow {
        /// How many times to repeat the delay series.
        #[arg(default_value_t = 15)]
        passes: usize,
    },
    /// Check whether the display follows refreshes after a rewind that Windows
    /// keeps.
    Freeze {
        /// Refreshes that advance the timestamp before the rewind.
        #[arg(default_value_t = FREEZE_BURST)]
        burst: u32,
    },
    /// Check whether a reverted write releases a display frozen by a rewind.
    RevertAfterFreeze {
        /// Refreshes that advance the timestamp before the rewind.
        #[arg(default_value_t = FREEZE_BURST)]
        burst: u32,
    },
    /// Check whether a held-preview refresh releases a display frozen by a
    /// rewind after explorer.exe restarts.
    ExplorerRestartAfterFreeze {
        /// Refreshes that advance the timestamp before the rewind.
        #[arg(default_value_t = FREEZE_BURST)]
        burst: u32,
    },
    /// Check whether the display follows refreshes after the Cloud timestamp is
    /// lowered.
    LowerCloud,
    /// Check whether the display follows refreshes after both timestamps are
    /// set to real time.
    ResetBoth,
    /// Check whether the display follows refreshes after ShellHost.exe
    /// restarts.
    RestartShellhost,
    /// Check whether the display follows refreshes after explorer.exe restarts.
    RestartExplorer,
    /// Check whether a refresh that holds preview on applies after an
    /// explorer.exe restart.
    HeldPreviewAfterExplorerRestart,
    /// Check whether a refresh that holds preview on applies during a freeze.
    HeldPreviewDuringFreeze {
        /// Refreshes that advance the timestamp before the rewind.
        #[arg(default_value_t = FREEZE_BURST)]
        burst: u32,
    },
    /// Check which preview timings apply a temperature while ordinary
    /// refreshes are blocked.
    PreviewTiming {
        /// What blocks ordinary refreshes before each trial.
        #[arg(value_enum)]
        blocker: Blocker,
        /// How many times to try each timing.
        #[arg(default_value_t = 2)]
        rounds: u32,
        /// Try only this timing, written as GAP/HOLD in milliseconds.
        #[arg(long, value_parser = cases::blocks::parse_timing)]
        timing: Option<(u64, u64)>,
    },
    /// Check whether switching Night Light off and on releases blocked
    /// refreshes.
    SwitchRelease {
        /// What blocks ordinary refreshes before the switch.
        #[arg(value_enum)]
        blocker: Blocker,
    },
    /// Check whether a freeze still blocks refreshes after explorer.exe
    /// restarts.
    FreezeSurvivesRestart,
    /// Write below Cloud twice, which Windows reverts, with Night Light off,
    /// switch it on, and check whether refreshes below and above the burst
    /// peak apply.
    RevertsWhileOff {
        /// Refreshes that advance the timestamp before the reverts.
        #[arg(default_value_t = FREEZE_BURST)]
        burst: u32,
        /// Move Cloud ahead of real time first, and wait for real time to
        /// pass the stored timestamp before switching on.
        #[arg(long)]
        cloud_ahead: bool,
    },
    /// Restart ShellHost.exe, then refresh and revert before and after Quick
    /// Settings opens, to trace when ShellHost.exe reads the settings.
    QuickSettingsReader,
    /// Run one part of the suite, which runs every case except watch in a
    /// fixed order.
    Suite {
        /// Which part to run, from 1 to 4.
        #[arg(value_parser = clap::value_parser!(u8).range(1..=i64::from(SUITE_PARTS)))]
        part: u8,
        /// The step of the part to start at.
        #[arg(long, default_value_t = 1)]
        from: usize,
    },
}

fn run(case: Case, observer: Observer) -> anyhow::Result<()> {
    match case {
        Case::Watch { seconds } => cases::watch(&observer, seconds),
        Case::Revert => Session::run("revert", observer, cases::revert::below_cloud),
        Case::CloudBoundary => {
            Session::run("cloud-boundary", observer, cases::revert::cloud_boundary)
        }
        Case::RevertWindow { passes } => Session::run("revert-window", observer, |session| {
            cases::revert::refresh_window(session, passes)
        }),
        Case::Freeze { burst } => Session::run("freeze", observer, |session| {
            cases::display::freeze(session, burst)
        }),
        Case::RevertAfterFreeze { burst } => {
            Session::run("revert-after-freeze", observer, |session| {
                cases::display::revert_after_freeze(session, burst)
            })
        }
        Case::ExplorerRestartAfterFreeze { burst } => {
            Session::run("explorer-restart-after-freeze", observer, |session| {
                cases::display::explorer_restart_after_freeze(session, burst)
            })
        }
        Case::LowerCloud => Session::run("lower-cloud", observer, cases::display::lower_cloud),
        Case::ResetBoth => Session::run("reset-both", observer, cases::display::reset_both),
        Case::RestartShellhost => Session::run(
            "restart-shellhost",
            observer,
            cases::display::restart_shellhost,
        ),
        Case::RestartExplorer => Session::run(
            "restart-explorer",
            observer,
            cases::display::restart_explorer,
        ),
        Case::HeldPreviewAfterExplorerRestart => Session::run(
            "held-preview-after-explorer-restart",
            observer,
            cases::display::held_preview_after_explorer_restart,
        ),
        Case::HeldPreviewDuringFreeze { burst } => {
            Session::run("held-preview-during-freeze", observer, |session| {
                cases::display::held_preview_during_freeze(session, burst)
            })
        }
        Case::PreviewTiming {
            blocker,
            rounds,
            timing,
        } => Session::run("preview-timing", observer, |session| {
            cases::blocks::preview_timing(session, blocker, rounds, timing)
        }),
        Case::SwitchRelease { blocker } => Session::run("switch-release", observer, |session| {
            cases::blocks::switch_release(session, blocker)
        }),
        Case::FreezeSurvivesRestart => Session::run(
            "freeze-survives-restart",
            observer,
            cases::blocks::freeze_survives_restart,
        ),
        Case::RevertsWhileOff { burst, cloud_ahead } => {
            Session::run("reverts-while-off", observer, |session| {
                cases::blocks::reverts_while_off(session, burst, cloud_ahead)
            })
        }
        Case::QuickSettingsReader => Session::run(
            "quick-settings-reader",
            observer,
            cases::quick_settings_reader,
        ),
        Case::Suite { part, from } => run_suite(part, from, &observer),
    }
}

fn run_suite(part: u8, from: usize, observer: &Observer) -> anyhow::Result<()> {
    let steps: Vec<Case> = SUITE
        .iter()
        .filter(|(step_part, _)| *step_part == part)
        .map(|(_, case)| *case)
        .collect();
    ensure!(
        (1..=steps.len()).contains(&from),
        "part {part} has steps 1 to {}",
        steps.len()
    );
    for (index, case) in steps.iter().enumerate().skip(from - 1) {
        let step = index + 1;
        say(&format!(
            "Suite part {part}, step {step} of {}: {case:?}",
            steps.len()
        ))?;
        run(*case, observer.clone()).with_context(|| {
            format!("suite part {part} stopped at step {step}; rerun it with --from {step}")
        })?;
        std::thread::sleep(SUITE_PAUSE);
    }
    say(&format!("Suite part {part} finished."))
}

fn main() -> anyhow::Result<ExitCode> {
    let case = Cli::parse().case;
    match run(case, Observer::spawn()) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) if error.is::<Quit>() => {
            say(&format!("{error:#}"))?;
            Ok(ExitCode::from(QUIT_EXIT_CODE))
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_suite_part_has_steps() {
        for part in 1..=SUITE_PARTS {
            assert!(SUITE.iter().any(|(step_part, _)| *step_part == part));
        }
    }

    #[test]
    fn suite_runs_only_mutating_cases() {
        assert!(
            SUITE
                .iter()
                .all(|(_, case)| !matches!(case, Case::Suite { .. } | Case::Watch { .. }))
        );
    }
}
