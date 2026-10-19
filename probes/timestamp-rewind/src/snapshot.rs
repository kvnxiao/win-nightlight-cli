//! Point-in-time readings of the Night Light settings and their `Cloud`
//! record.

use crate::registry;
use anyhow::Context as _;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::Settings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) read_at: SystemTime,
    pub(crate) settings_modified: u64,
    pub(crate) cloud_modified: u64,
    pub(crate) kelvin: Option<i16>,
    pub(crate) previewing: bool,
}

impl Snapshot {
    pub(crate) fn read(light: &Nightlight) -> anyhow::Result<Self> {
        let settings = light.read_settings()?;
        Ok(Self {
            read_at: SystemTime::now(),
            settings_modified: modified_secs(&settings)?,
            cloud_modified: registry::read_cloud_modified()?,
            kelvin: settings.color_temperature(),
            previewing: settings.is_previewing(),
        })
    }

    pub(crate) fn same_values(&self, other: &Self) -> bool {
        Self {
            read_at: other.read_at,
            ..self.clone()
        } == *other
    }

    pub(crate) fn to_json_since(&self, started: Instant) -> anyhow::Result<Value> {
        let mut value = self.to_json()?;
        if let Some(fields) = value.as_object_mut() {
            fields.insert(
                "elapsed_us".to_owned(),
                json!(started.elapsed().as_micros()),
            );
        }
        Ok(value)
    }

    pub(crate) fn to_json(&self) -> anyhow::Result<Value> {
        let now = unix_secs_f64(self.read_at)?;
        Ok(json!({
            "utc": utc(self.read_at)?,
            "settings_modified_unix_s": self.settings_modified,
            "settings_lead_s": self.settings_modified as f64 - now,
            "cloud_modified_unix_s": self.cloud_modified,
            "kelvin": self.kelvin,
            "previewing": self.previewing,
        }))
    }
}

/// Poll the settings every `interval` for `window` and return each reading
/// that differs from the one before it, starting with the first reading.
pub(crate) fn changes(
    light: &Nightlight,
    window: Duration,
    interval: Duration,
) -> anyhow::Result<Vec<Value>> {
    let started = Instant::now();
    let mut previous: Option<Snapshot> = None;
    let mut changes = Vec::new();
    while started.elapsed() < window {
        let snapshot = Snapshot::read(light)?;
        if previous
            .as_ref()
            .is_none_or(|before| !before.same_values(&snapshot))
        {
            changes.push(snapshot.to_json_since(started)?);
            previous = Some(snapshot);
        }
        std::thread::sleep(interval);
    }
    Ok(changes)
}

pub(crate) fn unix_secs(time: SystemTime) -> anyhow::Result<u64> {
    Ok(time.duration_since(UNIX_EPOCH)?.as_secs())
}

pub(crate) fn modified_secs(settings: &Settings) -> anyhow::Result<u64> {
    settings
        .modified()
        .context("stored settings timestamp is out of range")
        .and_then(unix_secs)
}

fn unix_secs_f64(time: SystemTime) -> anyhow::Result<f64> {
    Ok(time.duration_since(UNIX_EPOCH)?.as_secs_f64())
}

pub(crate) fn utc(time: SystemTime) -> anyhow::Result<String> {
    Ok(jiff::Timestamp::try_from(time)?.to_string())
}
