use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand, command};
use std::str::FromStr;
use win_nightlight_lib::{
    get_nightlight_settings, get_nightlight_state, nightlight_settings::ScheduleMode,
    set_nightlight_settings, set_nightlight_state,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut settings = get_nightlight_settings()
        .map_err(|e| anyhow!("Failed to read nightlight settings: {}", e))?;
    let mut state =
        get_nightlight_state().map_err(|e| anyhow!("Failed to read nightlight state: {}", e))?;

    match cli.command {
        Some(Commands::Temp { temperature }) => {
            if settings.set_color_temperature(temperature)? {
                set_nightlight_settings(&settings)?;
            }
        }
        Some(Commands::Schedule { mode }) => match mode {
            Schedule::Off => {
                if settings.set_mode(ScheduleMode::Off) {
                    set_nightlight_settings(&settings)?;
                }
            }
            Schedule::Solar => {
                if settings.set_mode(ScheduleMode::SunsetToSunrise) {
                    // Scheduled modes require nightlight state to be enabled
                    if !state.is_enabled {
                        state.enable();
                        set_nightlight_state(&state)?;
                    }
                    set_nightlight_settings(&settings)?;
                }
            }
            Schedule::Manual => {
                if settings.set_mode(ScheduleMode::SetHours) {
                    // Scheduled modes require nightlight state to be enabled
                    if !state.is_enabled {
                        state.enable();
                        set_nightlight_state(&state)?;
                    }
                    set_nightlight_settings(&settings)?;
                }
            }
        },
        Some(Commands::On) => {
            // Enables nightlight, ignoring any schedule mode
            if state.enable() {
                set_nightlight_state(&state)?;
            }
        }
        Some(Commands::Off) => {
            // Force disable nightlight, requires turning off any schedule mode as well
            if settings.set_mode(ScheduleMode::Off) {
                set_nightlight_settings(&settings)?;
            }
            if state.disable() {
                set_nightlight_state(&state)?;
            }
        }
        None => {
            println!("{:#?}", settings);
            println!("{:#?}", state);
        }
    }
    Ok(())
}
