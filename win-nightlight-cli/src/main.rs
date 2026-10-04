//! `wnl` command-line tool to turn Windows 11 Night Light on and off and
//! configure its schedule and color temperature.

use anyhow::Result;
use clap::CommandFactory;
use clap::Parser;
use clap::Subcommand;
use clap_complete::Generator;
use clap_complete::Shell;
use jiff::Timestamp;
use jiff::Zoned;
use jiff::tz::TimeZone;
use std::io;
use std::io::Write;
use std::time::SystemTime;
use win_nightlight_lib::ColorTemperature;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::Schedule;
use win_nightlight_lib::ScheduleMode;
use win_nightlight_lib::Settings;
use win_nightlight_lib::State;
use win_nightlight_lib::TimeOfDay;

const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S %Z";

/// Turn Windows 11 Night Light on and off and configure its schedule and color
/// temperature
#[derive(Debug, Parser)]
#[command(name = "wnl", version, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
enum Command {
    /// Show whether Night Light is on, the color temperature, and the schedule
    Status,
    /// Turn Night Light on until you or the schedule turn it off
    On,
    /// Turn Night Light off until you or the schedule turn it on
    ///
    /// Does not change the schedule. Use `wnl schedule off` to stop scheduled
    /// changes.
    Off,
    /// Turn Night Light on if it is off, or off if it is on
    Toggle,
    /// Set the color temperature
    ///
    /// Does not turn Night Light on.
    Temp {
        /// Color temperature in kelvin, from 1200 (warmest) to 6500 (no
        /// filter)
        #[arg(value_name = "KELVIN")]
        kelvin: ColorTemperature,
    },
    /// Set the schedule that turns Night Light on and off
    ///
    /// Changes only the schedule, then prints whether the current time is
    /// inside the new schedule's window.
    Schedule {
        #[command(subcommand)]
        schedule: ScheduleCommand,
    },
    /// Print a shell completion script
    Completions {
        /// Shell to generate the script for
        shell: Shell,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
enum ScheduleCommand {
    /// Stop turning Night Light on and off on a schedule
    ///
    /// Keeps the schedule type and times, and does not change whether Night
    /// Light is on.
    Off,
    /// Turn Night Light on at sunset and off at sunrise
    ///
    /// Windows computes sunset and sunrise from the device location.
    Sunset,
    /// Turn Night Light on and off at set times
    Hours {
        /// Time to turn Night Light on, as 24-hour HH:MM; keeps the stored
        /// time if omitted
        #[arg(long, value_name = "HH:MM")]
        start: Option<TimeOfDay>,
        /// Time to turn Night Light off, as 24-hour HH:MM; keeps the stored
        /// time if omitted
        #[arg(long, value_name = "HH:MM")]
        end: Option<TimeOfDay>,
    },
}

impl From<ScheduleCommand> for Schedule {
    fn from(command: ScheduleCommand) -> Self {
        match command {
            ScheduleCommand::Off => Self::Off,
            ScheduleCommand::Sunset => Self::SunsetToSunrise,
            ScheduleCommand::Hours { start, end } => Self::SetHours { start, end },
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli.command, &mut io::stdout().lock())
}

fn run(command: Command, out: &mut impl Write) -> Result<()> {
    let nightlight = Nightlight::new();
    match command {
        Command::Status => {
            let state = nightlight.read_state()?;
            let settings = nightlight.read_settings()?;
            render_status(&settings, &state, &TimeZone::system(), out)?;
        }
        Command::On => {
            nightlight.set_active(true)?;
        }
        Command::Off => {
            nightlight.set_active(false)?;
        }
        Command::Toggle => {
            nightlight.toggle()?;
        }
        Command::Temp { kelvin } => {
            nightlight.update_settings(|settings| settings.set_color_temperature(kelvin))?;
        }
        Command::Schedule { schedule } => {
            nightlight.update_settings(|settings| settings.set_schedule(schedule.into()))?;
            let settings = nightlight.read_settings()?;
            let now = Zoned::now();
            let now = TimeOfDay::new(u8::try_from(now.hour())?, u8::try_from(now.minute())?)?;
            render_schedule(&settings, now, out)?;
        }
        Command::Completions { shell } => {
            let mut command = Cli::command();
            command.set_bin_name("wnl");
            command.build();
            shell.try_generate(&command, out)?;
        }
    }
    Ok(())
}

fn render_status(
    settings: &Settings,
    state: &State,
    time_zone: &TimeZone,
    out: &mut impl Write,
) -> io::Result<()> {
    let timestamp = |time: Option<SystemTime>| {
        time.and_then(|time| Timestamp::try_from(time).ok())
            .map_or_else(
                || "unknown".to_owned(),
                |time| {
                    time.to_zoned(time_zone.clone())
                        .strftime(TIMESTAMP_FORMAT)
                        .to_string()
                },
            )
    };
    let yes_no = if state.is_active() { "yes" } else { "no" };

    writeln!(out, "active:            {yes_no}")?;
    writeln!(out, "transition cause:  {}", state.transition_cause())?;
    writeln!(
        out,
        "last transition:   {}",
        timestamp(state.last_transition())
    )?;
    writeln!(out, "state modified:    {}", timestamp(state.modified()))?;
    match settings.color_temperature() {
        Some(kelvin) => writeln!(out, "color temperature: {kelvin}K")?,
        None => writeln!(out, "color temperature: Windows default")?,
    }
    writeln!(out, "schedule:          {}", settings.mode())?;
    writeln!(
        out,
        "set hours:         {} to {}",
        settings.schedule_start(),
        settings.schedule_end()
    )?;
    writeln!(
        out,
        "sunset to sunrise: {} to {}",
        settings.sunset(),
        settings.sunrise()
    )?;
    writeln!(out, "settings modified: {}", timestamp(settings.modified()))
}

fn render_schedule(settings: &Settings, now: TimeOfDay, out: &mut impl Write) -> io::Result<()> {
    let mode = settings.mode();
    let (start, end) = match mode {
        ScheduleMode::Off => return writeln!(out, "schedule turned off"),
        ScheduleMode::SunsetToSunrise => (settings.sunset(), settings.sunrise()),
        ScheduleMode::SetHours => (settings.schedule_start(), settings.schedule_end()),
    };
    let position = if start == end {
        "this window is empty"
    } else if now.is_within(start, end) {
        "the current time is inside this window"
    } else {
        "the current time is outside this window"
    };
    writeln!(out, "schedule set to {mode}, {start} to {end}; {position}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_literal::hex;
    use test_case::test_case;

    const SETTINGS_SET_HOURS: &[u8] = &hex!(
        "43420100 0A020100 2A 06ECA0F4BE06 2A 2B0E26"
        "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF28CC2B CA320E132E1700 CA3C0E072E0C00 00"
        "00 00 00"
    );
    const SETTINGS_LIVE: &[u8] = &hex!(
        "43420100 0A020100 2A 06EFD487D606 2A 2B0E21"
        "43420100 CA140E1500 CA1E0E0700 CF28C435 CA320E132E1100 CA3C0E062E2600 00"
        "00 00 00"
    );
    const SETTINGS_TEMPERATURE_OUT_OF_RANGE: &[u8] = &hex!(
        "43420100 0A020100 2A 06ECA0F4BE06 2A 2B0E26"
        "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF28B06D CA320E132E1700 CA3C0E072E0C00 00"
        "00 00 00"
    );
    const SETTINGS_WITHOUT_TEMPERATURE: &[u8] = &hex!(
        "43420100 0A020100 2A 06E8DAFBF905 2A 2B0E15"
        "43420100 CA140E1500 CA1E0E0700 CA3200 CA3C00 00"
        "00 00 00"
    );
    const STATE_ACTIVE: &[u8] = &hex!(
        "43420100 0A020100 2A 068995FCBE06 2A 2B0E15"
        "43420100 1000 D00A02 C614A9F6E2D3EFEAE6ED01 00"
        "00 00 00"
    );
    const STATE_LIVE: &[u8] = &hex!(
        "43420100 0A020100 2A 069DED93CE06 2A 2B0E13"
        "43420100 D00A02 C614B09FD1E6F89FAFEE01 00"
        "00 00 00"
    );

    fn settings(data: &[u8]) -> Settings {
        Settings::from_bytes(data).expect("settings fixture decodes")
    }

    fn time(text: &str) -> TimeOfDay {
        text.parse().expect("fixture time is valid")
    }

    fn parse(args: &[&str]) -> Result<Command, clap::Error> {
        Cli::try_parse_from(std::iter::once("wnl").chain(args.iter().copied()))
            .map(|cli| cli.command)
    }

    fn render(write: impl FnOnce(&mut Vec<u8>) -> io::Result<()>) -> String {
        let mut out = Vec::new();
        write(&mut out).expect("writing to a vector succeeds");
        String::from_utf8(out).expect("output is UTF-8")
    }

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test_case(&["status"], Command::Status)]
    #[test_case(&["on"], Command::On)]
    #[test_case(&["off"], Command::Off)]
    #[test_case(&["toggle"], Command::Toggle)]
    #[test_case(&["completions", "powershell"], Command::Completions { shell: Shell::PowerShell })]
    #[test_case(&["schedule", "off"], Command::Schedule { schedule: ScheduleCommand::Off })]
    #[test_case(&["schedule", "sunset"], Command::Schedule { schedule: ScheduleCommand::Sunset })]
    fn parses_command(args: &[&str], expected: Command) {
        assert_eq!(parse(args).expect("arguments are valid"), expected);
    }

    #[test]
    fn parses_temperature() {
        assert_eq!(
            parse(&["temp", "3400"]).expect("arguments are valid"),
            Command::Temp {
                kelvin: ColorTemperature::new(3400).expect("3400K is in range")
            }
        );
    }

    #[test]
    fn parses_set_hours() {
        assert_eq!(
            parse(&["schedule", "hours", "--start", "21:00", "--end", "7:00"])
                .expect("arguments are valid"),
            Command::Schedule {
                schedule: ScheduleCommand::Hours {
                    start: Some(time("21:00")),
                    end: Some(time("07:00")),
                }
            }
        );
    }

    #[test_case(&["temp", "7000"]; "temperature above range")]
    #[test_case(&["temp", "warm"]; "temperature not a number")]
    #[test_case(&["temp"]; "temperature missing")]
    #[test_case(&["schedule", "sunset", "--start", "21:00"]; "time on sunset schedule")]
    #[test_case(&["schedule", "off", "--end", "07:00"]; "time on schedule off")]
    #[test_case(&["schedule", "hours", "--start", "25:00"]; "time out of range")]
    #[test_case(&["schedule", "solar"]; "old schedule name")]
    #[test_case(&["schedule"]; "schedule missing")]
    fn rejects_arguments(args: &[&str]) {
        let result = parse(args);
        assert!(result.is_err(), "{result:?}");
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn completions_write_failure_returns_error() {
        let result = run(
            Command::Completions {
                shell: Shell::PowerShell,
            },
            &mut FailingWriter,
        );
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn rejects_out_of_range_temperature_with_range_message() {
        let error = parse(&["temp", "7000"]).expect_err("7000K is out of range");
        assert!(
            error
                .to_string()
                .contains("color temperature 7000K is outside 1200K-6500K"),
            "{error}"
        );
    }

    #[test_case("live_inactive", SETTINGS_LIVE, STATE_LIVE)]
    #[test_case("set_hours_active", SETTINGS_SET_HOURS, STATE_ACTIVE)]
    #[test_case("without_temperature", SETTINGS_WITHOUT_TEMPERATURE, STATE_LIVE)]
    #[test_case(
        "temperature_out_of_range",
        SETTINGS_TEMPERATURE_OUT_OF_RANGE,
        STATE_ACTIVE
    )]
    fn renders_status(name: &str, settings_data: &[u8], state_data: &[u8]) {
        let settings = settings(settings_data);
        let state = State::from_bytes(state_data).expect("state fixture decodes");
        let output = render(|out| render_status(&settings, &state, &TimeZone::UTC, out));
        insta::assert_snapshot!(format!("status_{name}"), output);
    }

    #[test_case("set_hours_inside", ScheduleCommand::Hours { start: Some(time("21:00")), end: Some(time("07:00")) }, "23:30")]
    #[test_case("set_hours_outside", ScheduleCommand::Hours { start: Some(time("21:00")), end: None }, "12:00")]
    #[test_case("sunset_inside", ScheduleCommand::Sunset, "05:00")]
    #[test_case("off", ScheduleCommand::Off, "12:00")]
    fn renders_schedule(name: &str, command: ScheduleCommand, now: &str) {
        render_schedule_snapshot(name, SETTINGS_LIVE, command, now);
    }

    #[test]
    fn renders_schedule_with_uncomputed_sunset() {
        render_schedule_snapshot(
            "sunset_uncomputed",
            SETTINGS_WITHOUT_TEMPERATURE,
            ScheduleCommand::Sunset,
            "12:00",
        );
    }

    fn render_schedule_snapshot(name: &str, data: &[u8], command: ScheduleCommand, now: &str) {
        let mut settings = settings(data);
        settings.set_schedule(command.into());
        let output = render(|out| render_schedule(&settings, time(now), out));
        insta::assert_snapshot!(format!("schedule_{name}"), output);
    }
}
