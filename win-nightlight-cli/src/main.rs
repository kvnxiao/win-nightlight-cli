//! `wnl` command-line tool to toggle and configure Windows 11 Night Light.

use anyhow::Context;
use anyhow::Result;
use chrono::DateTime;
use chrono::Local;
use chrono::NaiveTime;
use clap::Parser;
use clap::Subcommand;
use indoc::printdoc;
use std::str::FromStr;
use win_nightlight_lib::NightlightManager;
use win_nightlight_lib::RegistryBackend;
use win_nightlight_lib::nightlight_settings::ScheduleMode;

const NAIVE_TIME_FORMAT: &str = "%I:%M %p";
const DATE_TIME_FORMAT: &str = "%Y-%m-%d %I:%M:%S %p %Z";

#[derive(Parser, Debug)]
#[command(name = "wnl", author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Schedule {
    Off,
    Solar,
    Manual,
}

impl FromStr for Schedule {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "off" => Schedule::Off,
            "solar" => Schedule::Solar,
            "manual" => Schedule::Manual,
            _ => anyhow::bail!("Valid modes are: 'off', 'solar', and 'manual'"),
        })
    }
}

impl From<Schedule> for ScheduleMode {
    fn from(s: Schedule) -> Self {
        match s {
            Schedule::Off => ScheduleMode::Off,
            Schedule::Solar => ScheduleMode::SunsetToSunrise,
            Schedule::Manual => ScheduleMode::SetHours,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Sets the color temperature in Kelvin (1200 - 6500)
    Temp {
        #[arg(index = 1)]
        temperature: u16,
    },
    /// Sets the schedule mode ('off', 'solar', or 'manual')
    Schedule {
        #[arg(index = 1)]
        mode: Schedule,
        /// Start time for 'manual' mode (HH:MM, 24-hour format)
        #[arg(long)]
        start: Option<String>,
        /// End time for 'manual' mode (HH:MM, 24-hour format)
        #[arg(long)]
        end: Option<String>,
    },
    /// Enables nightlight
    On,
    /// Disables nightlight
    Off,
    /// Prints the current nightlight state and settings
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mgr = NightlightManager::new(RegistryBackend);

    match cli.command {
        Commands::Temp { temperature } => mgr.set_color_temperature(temperature)?,
        Commands::Schedule { mode, start, end } => {
            let parse_time = |s: &str| -> Result<NaiveTime> {
                NaiveTime::parse_from_str(s, "%H:%M")
                    .with_context(|| format!("Invalid time format '{s}', expected HH:MM"))
            };

            let start_time = start.as_deref().map(parse_time).transpose()?;
            let end_time = end.as_deref().map(parse_time).transpose()?;

            mgr.set_schedule(mode.into(), start_time, end_time)?;
        }
        Commands::On => mgr.enable()?,
        Commands::Off => mgr.disable()?,
        Commands::Status => {
            let settings = mgr.get_settings()?;
            let state = mgr.get_state()?;

            let state_last_modified = i64::try_from(state.timestamp)
                .ok()
                .and_then(|secs| DateTime::from_timestamp(secs, 0))
                .context("Failed to convert timestamp to DateTime")?;
            let settings_last_modified = i64::try_from(settings.timestamp)
                .ok()
                .and_then(|secs| DateTime::from_timestamp(secs, 0))
                .context("Failed to convert timestamp to DateTime")?;
            let state_last_modified_local: DateTime<Local> = DateTime::from(state_last_modified);
            let settings_last_modified_local: DateTime<Local> =
                DateTime::from(settings_last_modified);

            printdoc!(
                r"
                Nightlight state:
                  - last modified:     {}
                  - is enabled:        {}

                Nightlight settings
                  - last modified:     {}
                  - color temperature: {}K
                  - schedule mode:     {}
                  - schedule start:    {}
                  - schedule end:      {}
                  - sunset time:       {}
                  - sunrise time:      {}
                ",
                state_last_modified_local.format(DATE_TIME_FORMAT),
                state.is_enabled,
                settings_last_modified_local.format(DATE_TIME_FORMAT),
                settings.color_temperature,
                settings.schedule_mode,
                settings.start_time.format(NAIVE_TIME_FORMAT),
                settings.end_time.format(NAIVE_TIME_FORMAT),
                settings.sunset_time.format(NAIVE_TIME_FORMAT),
                settings.sunrise_time.format(NAIVE_TIME_FORMAT),
            );
        }
    }
    Ok(())
}
