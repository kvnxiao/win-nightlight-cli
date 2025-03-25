use anyhow::{Result, anyhow};
use chrono::{DateTime, Local};
use clap::{Parser, Subcommand, command};
use indoc::printdoc;
use std::str::FromStr;
use win_nightlight_lib::{
    get_nightlight_settings, get_nightlight_state, nightlight_settings::ScheduleMode,
    set_nightlight_settings, set_nightlight_state,
};

const NAIVE_TIME_FORMAT: &str = "%I:%M %p";
const DATE_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S %Z";

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
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

#[derive(Subcommand, Debug)]
enum Commands {
    Temp {
        #[arg(index = 1)]
        temperature: u16,
    },
    Schedule {
        #[arg(index = 1)]
        mode: Schedule,
    },
    On,
    Off,
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut settings = get_nightlight_settings()
        .map_err(|e| anyhow!("Failed to read nightlight settings: {}", e))?;
    let mut state =
        get_nightlight_state().map_err(|e| anyhow!("Failed to read nightlight state: {}", e))?;

    match cli.command {
        Commands::Temp { temperature } => {
            if settings.set_color_temperature(temperature)? {
                set_nightlight_settings(&settings)?;
            }
        }
        Commands::Schedule { mode } => match mode {
            Schedule::Off => {
                if settings.set_mode(ScheduleMode::Off) {
                    set_nightlight_settings(&settings)?;
                }
            }
            Schedule::Solar => {
                if settings.set_mode(ScheduleMode::SunsetToSunrise) {
                    // Scheduled modes require nightlight state to be enabled
                    if state.enable() {
                        set_nightlight_state(&state)?;
                    }
                    set_nightlight_settings(&settings)?;
                }
            }
            Schedule::Manual => {
                if settings.set_mode(ScheduleMode::SetHours) {
                    // Scheduled modes require nightlight state to be enabled
                    if state.enable() {
                        set_nightlight_state(&state)?;
                    }
                    set_nightlight_settings(&settings)?;
                }
            }
        },
        Commands::On => {
            // Enables nightlight, ignoring any schedule mode
            if state.enable() {
                set_nightlight_state(&state)?;
            }
        }
        Commands::Off => {
            // Force disable nightlight, requires turning off any schedule mode as well
            if settings.set_mode(ScheduleMode::Off) {
                set_nightlight_settings(&settings)?;
            }
            if state.disable() {
                set_nightlight_state(&state)?;
            }
        }
        Commands::Status => {
            let state_last_modified = DateTime::from_timestamp(state.timestamp as i64, 0)
                .ok_or_else(|| anyhow!("Failed to convert timestamp to DateTime"))?;
            let settings_last_modified = DateTime::from_timestamp(settings.timestamp as i64, 0)
                .ok_or_else(|| anyhow!("Failed to convert timestamp to DateTime"))?;
            let state_last_modified_local: DateTime<Local> = DateTime::from(state_last_modified);
            let settings_last_modified_local: DateTime<Local> =
                DateTime::from(settings_last_modified);

            printdoc!(
                r#"
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
                "#,
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
