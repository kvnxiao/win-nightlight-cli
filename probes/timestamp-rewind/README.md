# Night Light timestamp rewind probe

This probe measures when Windows applies Night Light settings writes to the display after a program
moves the settings timestamp backward, and what else makes Windows ignore a temperature change. It
changes live Night Light settings and needs an observer at the display, who reports which tint the
screen shows. It runs only through this directory's `justfile`. The repository's root `justfile`
recipes build the probe and run its unit tests, but no test or CI job runs its cases.

## Summary

Each Night Light settings write stores a timestamp. `win-nightlight-lib`, the Rust library in this
repository, stamps each write at least 2 s after the stored timestamp. A program that changes the
temperature many times per second therefore moves the timestamp hours ahead of real time. This
probe started from the question of whether moving the timestamp back to real time resets that lead.

It does not. `explorer.exe`, the Windows process that applies Night Light to the display, ignores
temperature changes stamped at or below the highest timestamp it has applied. After the timestamp
moves back, the display stops changing until a temperature change is stamped above that old
maximum. For a program whose timestamps follow real time, a lead of hours means hours without
display changes. One method cleared the maximum in every run: a settings write stamped below the
timestamp of the latest Windows Settings write, which Windows replaces within milliseconds with that
Windows Settings write. This method has costs. Once, after it, the display ignored every settings
write until a sign-out, for an unexplained reason (finding 12).

Separately, after the probe ended `explorer.exe` and started a new one, Windows ignored the
library's temperature changes, whatever their timestamp. They applied again after Windows Settings
wrote the settings, or after a program kept the settings' preview flag on for about 9 ms or longer.

On 2026-10-19, a suite run repeated every case. Its questions listed all three tints instead of
naming the one each question checked for, and its results matched the earlier runs.

## Terms

- **Settings timestamp:** the last-modified time, in Unix seconds, in the outer structure (the
  envelope) of the Night Light settings value. The
  [registry format](../../docs/nightlight-registry-format.md#outer-cloudstore-envelope) describes
  the value and its envelope.
- **`win-nightlight-lib`:** the Rust library in this repository. Version 0.4.0 stamps each settings
  write with `max(now, previous + 2)`: the later of real time and 2 s after the stored timestamp.
- **Preview:** a flag in the settings value that asks Windows to apply the stored temperature
  temporarily, even while Night Light is off. Windows Settings turns it on when the Night Light
  Strength slider is pressed and off when the slider is released.
- **Refresh:** a sequence of settings writes that makes `explorer.exe` apply the stored temperature
  to the display. The probe makes two kinds:
  - **Ordinary refresh:** one call to the library's
    `Nightlight::set_color_temperature(t, RefreshPolicy::Always)`, which the library documentation
    calls a complete refresh. It writes temperature `t` with preview on, then preview off about
    0.2 ms later. Each ordinary refresh advances the settings timestamp by at least 4 s.
  - **Preview refresh `<gap>/<hold>`:** the probe turns preview on without changing the
    temperature, waits `gap` ms, writes the new temperature with preview still on, waits `hold` ms,
    and turns preview off. The probe restores the original temperature with a 50/50 preview
    refresh.
- **Lead:** how far the settings timestamp is ahead of real time. Each ordinary refresh advances the
  timestamp by at least 4 s, so refreshes made more often than once per 4 s build a lead: a
  1-second transition of refreshes at 144 Hz adds about 10 minutes.
- **Tints:** the three temperatures the probe writes, which an observer can tell apart at a glance:
  neutral (6500 K), yellow (3400 K), and deep red (1200 K).
- **Burst:** a series of ordinary refreshes to one tint, made back to back to build a lead. Cases
  with a burst make 256 refreshes by default.
- **`Cloud` timestamp:** the timestamp in the `Cloud` record, a separate registry value that stores
  only the timestamp of the latest Windows Settings write. The
  [registry format](../../docs/nightlight-registry-format.md#cloud-record) describes its location
  and layout.
- **Rewind:** a write that lowers the settings timestamp without changing any setting. The probe
  rewinds to the later of real time and `Cloud`.
- **Maximum:** the highest settings timestamp that `explorer.exe` has applied. In a case with a
  burst, it is the timestamp of the burst's last refresh.
- **Revert:** Windows replacing a settings write that is stamped below `Cloud` with the last
  Windows Settings write.
- **Stale-timestamp block:** the state after a rewind in which `explorer.exe` ignores ordinary
  refreshes stamped at or below the maximum.
- **Restart:** the probe ends `explorer.exe` with `taskkill /F` and starts a new `explorer.exe`.
- **Restart block:** the state after a restart in which `explorer.exe` ignores ordinary refreshes
  whatever their timestamp.
- **Block and release:** a block is a stale-timestamp block or a restart block. A release is an
  event after which ordinary refreshes apply again.
- **`ShellHost.exe`:** the Windows process that shows Quick Settings, the panel that Windows+A
  opens.
- **Slider write:** the settings writes that Windows Settings makes when the observer drags the
  Night Light Strength slider and releases it.
- **Case, suite, part, and step:** a case is one scenario the probe runs, started with
  `just <case>`; [Cases](#cases) lists them. The suite runs every case in four parts, one
  `just suite <part>` command per part. Each part runs its cases as numbered steps; step 2.1 is the
  first step of part 2.
- **Run, trial, and check:** a run is one execution of a case, which writes one results file. A
  trial is one repetition of a measured sequence inside a run, such as one preview timing. A check
  is one observer answer about whether a refresh applied.

## Findings

Counts cover every run on the [test machine](#test-machine) that checked the claim, including the
exploratory runs before the suite ([Runs before the suite](#runs-before-the-suite) describes
them). The repository does not include the results files or traces of these runs;
[Recheck the findings](#recheck-the-findings) describes how to reproduce them. Hyphenated names
in code font, such as `revert-window`, are cases from [Cases](#cases). Statements about what
`explorer.exe` keeps in memory are inferences from its behavior and from Process Monitor traces.

### 1. The `Cloud` timestamp changes only after Windows Settings writes

- `Cloud` is a 13-byte record with only a timestamp.
- `explorer.exe` writes it with the same timestamp 44–62 ms after each settings write by Windows
  Settings (`SystemSettings.exe`): 34 of 34 writes in the suite traces. It never wrote `Cloud`
  after a library write.
- Windows Settings stamps each write at least 2 s after the stored timestamp, as the library does.
  A slider drag during a lead therefore leaves `Cloud` ahead of real time.

### 2. Windows reverts settings writes stamped below `Cloud`

- A write 1 s below `Cloud` is replaced by the last Windows Settings write, with that write's
  timestamp and temperature. A write exactly at `Cloud` stays. Because `Cloud` stores no
  temperature, each reverting process writes the temperature from its own in-memory copy of the
  last Windows Settings write.
- `explorer.exe` always reverts. `ShellHost.exe` also reverts once it has started reading the
  settings ([finding 11](#11-shellhostexe-reads-the-settings-only-after-quick-settings-opens)).
- Each process usually reverts 2–4 ms after the write. Over 195 reverted writes in the suite's
  `revert-window` run, the median delay was 3.4 ms for `explorer.exe` and 2.6 ms for
  `ShellHost.exe`. In 31 of 195 writes for `explorer.exe`, and 10 of 195 for `ShellHost.exe`, the
  revert came 42–67 ms after the write.
- A revert overwrites any write made before it lands. In `revert-window`, ordinary refreshes sent
  after a write below `Cloud` were lost as follows, out of 15 at each delay:

  | Delay after the write below `Cloud` | Refreshes lost |
  | :---------------------------------- | :------------- |
  | 0 ms                                | 0              |
  | 1 ms, 2 ms                          | 15             |
  | 3 ms                                | 7              |
  | 5, 8, 12, 20, 30, 40 ms             | 3 at each      |
  | 50, 60, 80, 120 ms                  | 0              |

  At 0 ms, both processes read the refresh before reacting to the write below `Cloud`, so neither
  reverted.

### 3. A rewind at or above `Cloud` stays in the registry and causes a stale-timestamp block

- Windows kept every rewind to the later of real time and `Cloud`.
- After the rewind, `explorer.exe` ignored ordinary refreshes stamped at or below the maximum:
  52 of 53 checks across all runs, and 14 of 14 in the suite. The exception was an early trial:
  after a refresh below the maximum, the observer reported that the screen was not deep red, the
  tint it keeps if the refresh is ignored. That check is the one exception, and the case skipped
  the rest of that trial.
- A refresh stamped above the maximum applied every time.
- Setting `Cloud` and the settings timestamp to real time together still causes a stale-timestamp
  block (`reset-both`). Lowering `Cloud` alone does not affect refreshes (`lower-cloud`).

### 4. A write below `Cloud`, which Windows reverts, releases a stale-timestamp block

After a write below `Cloud` and its revert, ordinary refreshes 1,000–3,000 s below the old maximum
applied in 7 of 7 runs: three `revert-after-rewind` runs and four `reverts-while-off` runs. In the
first `reverts-while-off` run the evidence is indirect. Switching Night Light on showed the tint of
a refresh 1,025 s below the maximum, and switching on during a stale-timestamp block shows the last
tint applied
([finding 8](#8-switching-night-light-off-and-on-releases-neither-the-stale-timestamp-block-nor-the-restart-block)).
It is the only sequence of registry writes that released a stale-timestamp block in every run. Its
costs:

- The display shows the last Windows Settings temperature from the revert until the next refresh.
- A late revert can overwrite writes for up to about 70 ms ([finding 2](#2-windows-reverts-settings-writes-stamped-below-cloud)).
- It lowers the settings timestamp to `Cloud`, not to real time. `Cloud` is ahead of real time if
  Windows Settings wrote during a lead.
- In one run, after writes below `Cloud`, the display ignored every settings write until a sign-out
  ([finding 12](#12-in-one-run-the-display-ignored-settings-writes-until-a-sign-out-for-an-unknown-reason)).
  The cause is unknown.

### 5. Whether a preview refresh releases a stale-timestamp block depends on when `explorer.exe` reads the settings

- For an ordinary refresh, `explorer.exe` reads only the preview-off value, because preview turns
  off about 0.2 ms after it turns on.
- These counts include only trials in which the observer first confirmed the stale-timestamp
  block. A 0/0 preview refresh makes its three writes within about 0.5 ms. It released the block in
  13 of 15 trials, and 5 of 5 in the suite. Every other timing, including holds of 100 ms to 2 s in
  an earlier version of the case, released it in 2 of 24 trials. None of the 10 trials with a
  50 ms gap released it. A preview held for 2–3 s released it in one of two runs
  (`held-preview-after-rewind`).
- Process Monitor traced 28 trials with a stale-timestamp block. In the 12 that released the
  block, `explorer.exe` read a preview-on value and then its next value 0.4–0.7 ms later. In the 16
  that did not, the next read came about 50 ms later, or `explorer.exe` did not read a preview-on
  value.
- A writer cannot see which values `explorer.exe` read. A failed attempt leaves the block in place
  and the registry in the same state as a successful one, so a writer cannot rely on a 0/0 preview
  refresh to release a stale-timestamp block.

### 6. An `explorer.exe` restart ends a stale-timestamp block but causes a restart block

- A restarted `explorer.exe` applies the stored temperature at startup, even when its timestamp is
  below the old maximum: 5 of 5 runs (`stale-timestamp-survives-restart`,
  `explorer-restart-after-rewind`).
- After a preview refresh releases the restart block, ordinary refreshes stamped 49 s to about
  6,000 s below the old maximum applied in 4 of 4 runs.

### 7. Every `explorer.exe` restart blocks ordinary refreshes

- After each restart, `explorer.exe` ignored ordinary refreshes whatever their timestamp: 31 of 31
  checks across all runs. Signing out and back in does not cause the block: ordinary refreshes
  applied in the runs after the sign-out in finding 12.
- Opening the Night Light page in Windows Settings did not release the block (2 runs). A slider
  write released it (3 runs).
- A preview refresh releases the block when preview stays on long enough. Process Monitor traced
  20 trials. The block was released in all 15 in which `explorer.exe` read preview on and preview
  off at least 12.6 ms apart, and in none of the 5 in which the two reads were at most 8.2 ms
  apart.
- By the probe's own clock, from its preview-on write to its preview-off write, previews kept on
  for 8.6 ms or longer released the block, and previews kept on for 7.0 ms or less did not.
  Keeping preview on for about 9 ms or longer therefore released the block in these measurements.
- Previews held for several seconds released the block in 4 of 4 runs. Ordinary refreshes applied
  after each release.
- An ordinary refresh turns preview off about 0.2 ms after turning it on, so it cannot release the
  block.
- No preview timing released both blocks. In the traced trials, a stale-timestamp block was
  released only when `explorer.exe` read the next value within 0.7 ms of a preview-on value, and a
  restart block only when the two reads were more than 8.2 ms apart.

### 8. Switching Night Light off and on releases neither the stale-timestamp block nor the restart block

- During a stale-timestamp block, switching on showed the last tint the display applied, not the
  stored tint, and the next ordinary refresh was ignored (2 runs).
- After a restart, switching on showed the stored tint, and the next ordinary refresh was ignored
  (2 runs).

### 9. Without a block, preview refreshes apply with each of the four tested timings

With neither block in place, preview refreshes with all four timings (0/0, 0/50, 50/0, and 50/50)
applied, and ordinary refreshes after them applied: 8 of 8 trials in 2 runs. A 50/50 preview
refresh therefore applies both without a block and after a restart, which is why the probe restores
the original temperature with one.

### 10. Restarting `ShellHost.exe` does not affect refreshes

An ordinary refresh after a `ShellHost.exe` restart applied in 3 of 3 runs.

### 11. `ShellHost.exe` reads the settings only after Quick Settings opens

A newly started `ShellHost.exe` did not read the settings or revert writes until Quick Settings
opened. From then on, it read every change and reverted writes below `Cloud`, also after Quick
Settings closed. This held in 3 traced `quick-settings-reader` runs and in suite parts 1 and 4,
where the observer opened Windows Settings through Quick Settings. Windows also ended and
relaunched `ShellHost.exe` once during a run without logging a crash. The new process did not read
the settings.

### 12. In one run, the display ignored settings writes until a sign-out, for an unknown reason

The first version of this probe ran four times on 2026-10-18. Each run made a sweep and later
rewound. A sweep was 257 ordinary refreshes at up to 144 Hz that stepped the temperature from
6500 K down to 2400 K and back up, about 30 K per refresh. Only run 1 made a second sweep after its
rewind.

In the third run, the display already ignored the first sweep,
and from then on it ignored every settings write. This included writes stamped above every earlier
maximum. When the observer used the Strength slider in Windows Settings, pressing the slider, which
turns preview on, changed the display, but dragging and releasing it did not. Signing out and back
in cleared the state. The Windows Application log shows no `explorer.exe` crash in that period.

The second run, the last one before that state, had these
conditions:

- A sweep.
- Two writes below `Cloud` that Windows reverted, the second with Night Light off.
- After each revert, the probe wrote back the settings and on/off state from the start of the run.
  `Cloud` was about 340 s ahead of real time, and each revert set the settings timestamp to
  `Cloud`, so the write after the first revert and the write after the second revert had the same
  timestamp.
- The third run started 12 minutes after the second ended.

Four replays did not reproduce the state:

- Two `reverts-while-off` runs repeated a burst and the two reverts with Night Light off.
- Two `reverts-while-off --cloud-ahead` runs also moved `Cloud` 134 s and 439 s ahead of real time,
  so the refresh after the first revert and the refresh after the second revert had the same
  timestamp. They waited up to about 7 minutes before switching on.

None of the replays made a sweep or repeated the exact writes of the first probe version.

## What this means for programs that write Night Light settings

- **Resetting a timestamp lead:** a rewind alone causes a stale-timestamp block that lasts until a
  refresh is stamped above the old maximum. For a program whose timestamps follow real time, a lead
  of hours means hours of ignored refreshes. A write below `Cloud` cleared the maximum in every run
  but has the costs in
  [finding 4](#4-a-write-below-cloud-which-windows-reverts-releases-a-stale-timestamp-block). The
  probe does not show that it is safe, because the state in finding 12 is unexplained. A rewind
  followed by a 0/0 preview refresh releases the stale-timestamp block only when `explorer.exe`
  reads the values within about 0.7 ms (finding 5).
- **Any writer:** keep the settings timestamp increasing, as `max(now, previous + 2)` does. After a
  write that Windows reverts, wait at least 70 ms before the next write.
- **`win-nightlight-lib` after an `explorer.exe` restart:** ordinary refreshes are ignored until a
  slider write or a preview that stays on long enough. A 50/50 preview refresh applies both without
  a block and after a restart. It costs a third settings write, which adds 2 s of lead per
  refresh, and about 100 ms per refresh. This probe does not change the library.

## Cases

Run a case with `just <case> [arguments]`. Each case that asks about the screen first makes a
neutral and then a red ordinary refresh, asks about each, and stops if the display does not follow
them. The Suite column gives the part and step of the suite that run the case.

| Case                                    | What it does                                                                                                                                                                                                 | Findings  | Suite              |
| :-------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :-------- | :----------------- |
| `revert`                                | Writes the settings 1 s below `Cloud` and records every change to the settings and `Cloud` values in the next second.                                                                                       | 2         | 1.2                |
| `cloud-boundary`                        | Writes the settings at `Cloud`, then 1 s below it, and records whether each write stays.                                                                                                                    | 2         | 1.3                |
| `revert-window [passes]`                | Runs 14 delays per pass, 15 passes by default: writes below `Cloud`, waits 0–120 ms, makes an ordinary refresh, and checks whether it survived.                                                             | 2         | 1.4                |
| `stale-timestamp [burst]`               | Makes a burst, rewinds to the later of real time and `Cloud`, then refreshes below the maximum and above it.                                                                                                | 3         | 2.1                |
| `revert-after-rewind [burst]`           | Causes a stale-timestamp block as `stale-timestamp` does, writes below `Cloud`, then refreshes below the maximum.                                                                                           | 3, 4      | 1.6                |
| `held-preview-after-rewind [burst]`     | Causes a stale-timestamp block, holds a preview until the observer answers, then refreshes below the maximum.                                                                                               | 5         | 2.2                |
| `preview-timing <blocker> [rounds]`     | Tries the 0/0, 0/50, 50/0, and 50/50 timings in each of `rounds` rounds, 2 by default. Each trial sets up the blocker (`none`, `stale-timestamp`, or `explorer-restart`), checks the screen, makes the preview refresh, then an ordinary refresh. `--timing <gap>/<hold>` tries only that timing. | 5, 7, 9   | 2.4, 2.5, 3.1, 3.6 |
| `switch-release <blocker>`              | Blocks refreshes, switches Night Light off and on, then refreshes.                                                                                                                                           | 8         | 2.3, 3.5           |
| `stale-timestamp-survives-restart`      | Causes a stale-timestamp block, restarts `explorer.exe`, makes a 50/50 preview refresh, then refreshes below the old maximum.                                                                               | 6         | 3.4                |
| `explorer-restart-after-rewind [burst]` | Causes a stale-timestamp block, restarts `explorer.exe`, refreshes, holds a preview, then refreshes below the old maximum.                                                                                  | 6, 7      | 4.1                |
| `held-preview-after-explorer-restart`   | Restarts `explorer.exe`, refreshes, holds a preview, then refreshes.                                                                                                                                         | 7         | 4.2                |
| `restart-explorer`                      | Restarts `explorer.exe` and refreshes. If that refresh is ignored, asks the observer to open the Night Light page and refreshes; if that is ignored too, asks for a slider write and refreshes.             | 7         | 4.5                |
| `restart-shellhost`                     | Restarts `ShellHost.exe` and refreshes.                                                                                                                                                                      | 10        | 3.3                |
| `quick-settings-reader`                 | Restarts `ShellHost.exe`, refreshes and writes below `Cloud`, asks the observer to open and close Quick Settings, then refreshes and writes below `Cloud` again. Run it under `just trace`: the trace summary shows when `ShellHost.exe` starts reading the settings. | 2, 11     | 3.2                |
| `lower-cloud`                           | Lowers `Cloud` by 600 s and refreshes.                                                                                                                                                                       | 3         | 4.3                |
| `reset-both`                            | Makes a burst of 64 refreshes, sets `Cloud` and the settings timestamp to real time, then refreshes below the maximum and above it.                                                                         | 3         | 4.4                |
| `reverts-while-off [burst]`             | Makes a burst, then twice writes below `Cloud`, refreshes, and switches Night Light off. Switches it on and refreshes below and above the maximum. If the refresh above the maximum is ignored, tries these releases in order until one works: a 50/50 preview refresh, switching Night Light off and on, a slider write, and a restart followed by a preview refresh. | 4, 12     | 1.5                |
| `reverts-while-off [burst] --cloud-ahead` | Adds a slider write before the burst, which moves `Cloud` ahead of real time, and waits for real time to pass the stored timestamp before switching on.                                                  | 4, 12     | 1.1                |
| `watch [seconds]`                       | Prints each change to the settings and `Cloud` timestamps without writing.                                                                                                                                  | —         | —                  |

## Run the probe

### Prerequisites

- Windows 11 with Night Light, from the interactive user's desktop, with a stored color
  temperature from 1200 to 6500 K.
- Rust stable, `just`, Git, and Windows PowerShell.
- For traces: [Process Monitor](https://learn.microsoft.com/sysinternals/downloads/procmon) on
  `PATH` as `procmon`, and [`uv`](https://docs.astral.sh/uv/), which runs the Python trace tools
  in the `trace/` project. Process Monitor asks for administrator approval when a capture starts
  and when it ends.
- Close Windows Settings. Each case refuses to start while a Windows Settings window is open or
  preview is on, because either one writes the settings during the case.
- Stop other programs that write Night Light settings, such as scripts that run `wnl`, this
  repository's command-line app, in a loop. The probe does not detect them.

### Commands

```bash
just <case> [arguments]               # run one case
just trace <seconds> <case> [args]    # run one case under a Process Monitor capture
just trace 3600 suite <part>          # run suite part 1, 2, 3, or 4 under one capture
just trace 3600 suite <part> --from 3 # resume a part at step 3
just environment                      # print the Windows build, GPUs, and display refresh rates
```

The capture ends 3 s after the case exits, or after `seconds`, whichever comes first. The recipe
prints a warning if `seconds` ran out first. `just trace <seconds>` without a case captures until
`seconds` pass or the observer types `quit`.

### Answer the probe's questions

Each question lists the tints instead of naming the one it checks for. Type `1` for neutral
(6500 K), `2` for yellow (3400 K), `3` for deep red (1200 K), or `0` for anything else or when
unsure. The probe discards lines other than `quit` that were typed before a question or instruction
appeared, so a line typed early does not answer it. Type `quit` to stop: the probe restores Night
Light and exits with code 3, which the recipes treat as success.

### Run every case with the suite

`suite <part>` runs every case except `watch` in four parts, and writes one results file per case.
Parts 2 to 4 take about 2 minutes each. Part 1 takes longer because of the wait below. The order
keeps cases from changing each other's conditions:

- The `--cloud-ahead` replay runs first, while the settings timestamp is close to real time,
  because it waits for real time to pass that timestamp. It prints how long it will wait.
- Cases that write `Cloud`, or ask for a slider write, run last, because they change the timestamp
  that reverts restore.

A part stops when a step fails, for example when the display does not follow the first neutral and
red refreshes, or when the observer types `quit`. It then prints the `--from` step to resume at.

### Changes a run leaves behind

- Each case changes the temperature and the on/off state, then restores both. Restoring writes a
  settings timestamp above every timestamp the case saw, so cases with a burst leave the lead about
  1,000 s larger. The full suite leaves a lead of about 6,000 s.
- A slider write during a lead leaves `Cloud` ahead of real time by that lead.
- Cases that restart `explorer.exe` hide the taskbar for a few seconds.
- `revert-window` makes the screen flicker for about 2 minutes.
- Killing the probe, for example with Ctrl+C, skips restoring. If Night Light then ignores
  temperature changes, restart `explorer.exe` and move the Strength slider in Windows Settings, or
  sign out and back in.
- The `reverts-while-off` cases replay the conditions of the state in finding 12, which needed a
  sign-out.

## Results and traces

Each case writes `results/<case>-<unix seconds>.jsonl`, one JSON record per line:

- `environment`: the Windows build, GPUs, display refresh rates, Git revision and status of the
  probe, and `rustc --version`.
- Snapshots, such as `start`, `burst`, and `restored`: the settings and `Cloud` timestamps, the
  lead, the temperature, the preview flag, and the `explorer.exe` and `ShellHost.exe` process IDs.
- `observer`: the question's label, the tint the question checks for, and the observer's answer. A
  question about a refresh checks for that refresh's tint. A question about a block checks for the
  tint that stays on screen if the block holds.
- Records specific to a case, such as `trial_outcome` or `refresh_after_revert`.

`just trace` writes `traces/trace-<unix seconds>.pml` and its summary `.tsv`. Captures reach about
90 MB. Git ignores both `results/` and `traces/`. Each summary line has, separated by tabs:

1. Milliseconds since the first event.
2. Process name.
3. Process ID.
4. `read` or `write`.
5. The key: `Current` or `Cloud`, and `settings` or `state`.
6. The settings timestamp in the value.
7. The value length in bytes. Process Monitor records only the first 16 bytes of a value, so the
   length is the only sign of the preview flag: a value with preview on is 3 bytes longer.
8. The UTC time of the event.

Two commands of the `trace/` project read the summaries:

```bash
# What explorer.exe read during each preview refresh (findings 5 and 7)
uv run --locked --project trace preview-reads results/preview-timing-<n>.jsonl traces/trace-<m>.tsv
# Which processes reverted each write below Cloud, and how fast (finding 2)
uv run --locked --project trace reverts traces/trace-<m>.tsv [results/<case>-<n>.jsonl]
```

`trace/` is a uv project with Python 3.15. `just trace-check` runs its lint, type-check, test,
and dependency audit tasks, which CI also runs. `just trace-fix` applies lint fixes and formatting.

### Suite run on 2026-10-19

The suite ran on 2026-10-19 from 02:10 to 02:28 UTC. Of 134 answers, 115 named the tint the
question checked for. All 19 others answered questions that check whether a refresh applied, where
a finding predicts the refresh is ignored, and each named the tint already on screen.

### Recheck the findings

Run the four suite parts under `just trace`, as [Commands](#commands) shows. Then, for each case,
compare its results file and the trace summary with the findings that [Cases](#cases) lists for
it. `preview-reads` and `reverts` print the trace measurements for findings 2, 5, and 7. Results
can differ on another Windows build, display, or observer; the `environment` record at the start
of each results file names the build and display.

### Runs before the suite

- 2026-10-18 from 21:46 to 22:41 UTC: the four runs of the first version of this probe, described
  in finding 12. Its code is no longer in the repository. Run 1 reported that its second sweep,
  stamped below the first sweep's maximum, changed the display. The later cases write tints that
  differ clearly from each other, and they show that such refreshes are ignored (finding 3), so
  that report is treated as misjudged. Run 3 already showed the state in finding 12, and run 4
  followed the sign-out.
- 2026-10-18 23:33 UTC to 2026-10-19 01:58 UTC: exploratory runs of the current cases and their
  earlier versions. Their questions named the tint they checked for and took yes or no. Four of
  these runs came from an earlier version of `preview-timing` that varied only the hold, and three
  of those four stopped before their first measurement.

## Test machine

Measured on 2026-10-18 and 2026-10-19 with `just environment`; each results file records the same
fields:

- Windows 11, build 26300.9711.
- NVIDIA GeForce RTX 5090, driver 32.0.16.1742, one active display at 2560×1440 and 479.989 Hz.
- A virtual display adapter installed by the MuMuPlayer Android emulator, with no active display.
- Rust 1.99.0 stable, debug profile.

## Limits and open questions

- All runs used one machine, one Windows build, one display, and one observer.
- Windows does not document any of this behavior. The maximum that causes a stale-timestamp block,
  the restart block, and the in-memory copies are inferred from what `explorer.exe` applied and from
  registry traces.
- Answers are subjective. The suite's questions did not name the tint they checked for, but the
  earlier runs' questions did.
- Not tested:
  - A slider write stamped below the maximum during a stale-timestamp block. Every slider write
    observed during a stale-timestamp block was stamped above it.
  - Preview timings between 1 and 50 ms during a stale-timestamp block.
  - Whether an `explorer.exe` that Windows restarts after a crash blocks refreshes as a restart by
    the probe does.
  - Whether system load changes the revert delays.
- Finding 12 is unexplained. The replays did not include the first probe version's sweep of
  changing temperatures at 144 Hz.
- Process Monitor deleted some early capture files, for an unknown reason. Since the trace recipe
  started renaming each capture after it ends, no capture has been lost.
