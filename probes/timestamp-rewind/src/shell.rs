//! Windows shell processes that read Night Light settings, and helper
//! scripts.

use anyhow::Context as _;
use anyhow::ensure;
use camino::Utf8Path;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

const RELAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
const RELAUNCH_POLL: Duration = Duration::from_millis(200);

/// A Windows shell process that the probe can end and restart.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ShellProcess {
    Explorer,
    ShellHost,
}

impl ShellProcess {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Explorer => "explorer.exe",
            Self::ShellHost => "ShellHost.exe",
        }
    }

    pub(crate) fn ids(self) -> anyhow::Result<Vec<u32>> {
        let filter = format!("IMAGENAME eq {}", self.name());
        let listing = capture("tasklist", &["/FO", "CSV", "/NH", "/FI", &filter])?;
        listing
            .lines()
            .filter(|line| line.starts_with('"'))
            .map(|line| {
                line.split(',')
                    .nth(1)
                    .context("tasklist row has no process ID")?
                    .trim_matches('"')
                    .parse()
                    .context("tasklist process ID is not a number")
            })
            .collect()
    }

    /// End every instance and wait until a new instance runs.
    ///
    /// Windows relaunches `ShellHost.exe` on its own; the probe starts a new
    /// `explorer.exe` because Windows does not relaunch a terminated shell.
    pub(crate) fn restart(self) -> anyhow::Result<Vec<u32>> {
        let before = self.ids()?;
        capture("taskkill", &["/F", "/IM", self.name()])?;
        if matches!(self, Self::Explorer) {
            Command::new("explorer.exe")
                .spawn()
                .context("starting explorer.exe")?;
        }
        let started = Instant::now();
        loop {
            let ids = self.ids()?;
            if !ids.is_empty() && ids.iter().all(|id| !before.contains(id)) {
                return Ok(ids);
            }
            ensure!(
                started.elapsed() < RELAUNCH_TIMEOUT,
                "{} did not relaunch within {} s",
                self.name(),
                RELAUNCH_TIMEOUT.as_secs()
            );
            std::thread::sleep(RELAUNCH_POLL);
        }
    }
}

pub(crate) fn capture(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    ensure!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub(crate) fn powershell_script(name: &str) -> anyhow::Result<String> {
    let script = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    capture(
        "powershell.exe",
        &[
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            script.as_str(),
        ],
    )
}
