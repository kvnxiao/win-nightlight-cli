# Night Light timestamp rewind probe

This interactive probe tests when Night Light settings writes change the display. It covers
backward timestamps, `explorer.exe` restarts, preview timing, fades, and interference from the Strength
slider in Windows Settings.

**The probe changes live Night Light settings and needs an observer at the display.** Run its cases
through this directory's `justfile`. The root tasks and CI only build it and run its unit tests.

## Summary

Writing temperatures frequently moves the settings timestamp ahead of real time. Moving it backward
does not reliably undo that effect: `explorer.exe` can continue ignoring writes below a previously applied
timestamp, even though the registry accepts them.

The probe found:

- **A rewind can stop display updates.** Writing below the separate `Cloud` timestamp caused Windows
  to restore an earlier value and release this block, but could overwrite intervening changes
  (findings 3 and 4). Restarting `explorer.exe` also cleared the stale-timestamp block (finding 6).
- **Forced `explorer.exe` restarts caused a different block.** Plain writes and previews lasting about
  0.2 ms were ignored. Slider writes or previews lasting about 9 ms or longer restored updates in
  the measured trials (finding 7).
- **Held previews supported gradual fades.** Fades looked gradual at all tested rates, from 30 to
  480 writes per second, including after an `explorer.exe` restart (findings 13 and 14).
- **Fast fades interfered with the slider.** Most slider writes had timestamps below the fade's
  latest writes. Visual observations showed the slider's temperature only briefly; later runs recorded timestamps
  without observing that part of the fade (finding 15).
- **Plain writes worked in normal conditions.** They were ignored after a restart or a timestamp rewind
  in the tested cases (finding 16).

All measurements came from one machine, Windows build, display, and observer. They do not establish
Windows API guarantees. Suite parts 1–4 repeated the earlier timestamp and restart experiments;
parts 5–6 tested fades and plain writes.

- For application guidance, read [Color Temperature Transitions](../../docs/color-temperature-transitions.md).
- To reproduce a result, see [Run the probe](#run-the-probe) and [Cases](#cases).
- For the measurements and uncertainties, see [Findings](#findings) and [Limits and open questions](#limits-and-open-questions).

## Terms

- **Settings timestamp:** the last-modified time, in Unix seconds, in the outer structure (the
  envelope) of the Night Light settings value. The
  [registry format](../../docs/nightlight-registry-format.md#outer-cloudstore-envelope) describes
  the value and its envelope.
- **`win-nightlight-lib`:** the Rust library in this repository. It stamps each settings write with
  `max(now, previous + 2, highest + 2)`: the latest of real time, 2 s after the stored timestamp,
  and 2 s after the highest timestamp the same library handle has written. After each write that
  moves the timestamp back, the probe replaces its handle, so its later writes follow the stored
  timestamp, as the writes of a newly started program do. The runs for findings 1 to 12, including
  suite parts 1 to 4, used version 0.4.0. The runs for findings 13 to 16 used version 0.5.0, which
  adds the methods named below. The 0.5.0 runs from 06:57 to 07:57 UTC on 2026-10-19 used an earlier
  build that stamped `max(now, previous + 2)`; the runs from 18:35 UTC on used the current rule.
- **Preview:** a flag that asks Windows to apply the stored temperature temporarily, even while
  Night Light is off. Windows Settings normally turns it on when the Strength slider is pressed
  and off when released. Finding 15 records a release without a preview-off write during a
  concurrent fade.
- **Plain write:** one settings write that changes the temperature without preview, as
  `Nightlight::set_color_temperature` makes.
- **Fade:** a temperature change made of many small steps, here usually from neutral to red. The
  [color temperature transitions guide](../../docs/color-temperature-transitions.md) calls a
  gradual change longer than 30 s a ramp.
- **Refresh:** a sequence of settings writes meant to make `explorer.exe` apply the stored
  temperature to the display. The probe makes two kinds:
  - **Ordinary refresh:** a write of temperature `t` with preview on, then a write that turns
    preview off about 0.2 ms later. The probe makes it with `Nightlight::preview_color_temperature`
    and `Nightlight::end_preview`. Version 0.4.0 of the library made the same two writes with
    `set_color_temperature(t, RefreshPolicy::Always)`, which the runs for findings 1 to 12 used.
    Version 0.5.0 removed that form; its `set_color_temperature(t)` makes a plain write. Each
    ordinary refresh advances the settings timestamp by at least 4 s.
  - **Preview refresh `<gap>/<hold>`:** the probe turns preview on without changing the
    temperature, waits `gap` ms, writes the new temperature with preview still on, waits `hold` ms,
    and turns preview off. The probe restores the original temperature with a 50/50 preview
    refresh.
- **Held drag:** a fade made with the preview methods of `win-nightlight-lib` 0.5.0, the way
  dragging the Strength slider makes one. `start_preview` turns preview on, one
  `preview_color_temperature` call per step writes the next temperature at a fixed rate, and
  `end_preview` turns preview off. Each write advances the settings timestamp by at least 2 s.
- **Per-step drag:** one `preview_color_temperature` call and, 50 ms later, one `end_preview` call.
  A click on the slider is similar: Windows Settings first writes the temperature without preview,
  then keeps preview on for 51 to 97 ms. A per-step drag advances the timestamp by at least 4 s.
  `Nightlight::apply_color_temperature` makes the same writes while Night Light is on; while it is
  off, it makes one plain write that also turns preview off.
- **Lead:** how far the settings timestamp is ahead of real time. Each ordinary refresh advances the
  timestamp by at least 4 s, so refreshes made more often than once per 4 s build a lead: a
  1-second transition of refreshes at 144 Hz adds about 10 minutes.
- **Tints:** the three temperatures the probe writes, which an observer can tell apart at a glance:
  neutral (6500 K), yellow (3400 K), and deep red (1200 K).
- **Burst:** a series of ordinary refreshes to one tint, made back to back to build a lead. Cases
  with a burst make 256 refreshes by default.
- **`Cloud` timestamp:** the timestamp in the `Cloud` record, a separate registry value that stores
  only a timestamp from a settings write made by Windows, usually Windows Settings and once an
  `explorer.exe` startup write in the traces. The
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
  `just <case>`; [Cases](#cases) lists them. The suite runs every case except `watch` in six
  parts, one `just suite <part>` command per part. Each part runs its cases as numbered steps;
  step 2.1 is the first step of part 2.
- **Run, trial, and check:** a run is one execution of a case, which writes one results file. A
  trial is one repetition of a measured sequence inside a run, such as one preview timing. A check
  is one observer answer about whether a refresh applied.

## Findings

Counts cover every run on the [test machine](#test-machine) that checked the claim, including the
exploratory runs before the suite ([Runs before the suite](#runs-before-the-suite) describes
them). The repository does not include the results files or traces of these runs;
[Recheck the findings](#recheck-the-findings) describes how to reproduce them. In the findings,
hyphenated names in code font, such as `revert-window`, are cases from [Cases](#cases). Statements
about what `explorer.exe` keeps in memory are inferences from its behavior and from Process Monitor
traces.

<a id="1-the-cloud-timestamp-changes-after-windows-settings-writes-never-after-library-writes"></a>

### 1. Cloud updates followed writes made by Windows

- `Cloud` is a 13-byte record with only a timestamp. The state value has a `Cloud` record with the
  same layout.
- `explorer.exe` wrote the settings' `Cloud` record 594 times in the traces. In 593 of them, it
  wrote the timestamp of a settings write by Windows Settings (`SystemSettings.exe`), up to 78 ms
  after that write. Once, a restarted `explorer.exe` wrote the settings itself at startup, stamped
  2 s after the stored timestamp, and wrote that timestamp to `Cloud` 3.3 s later. `explorer.exe`
  never wrote `Cloud` after a library write.
- The state's `Cloud` record follows the same pattern. Windows Settings and Quick Settings
  (`ShellHost.exe`) stamped each state write that followed an earlier state write in the same trace
  with the later of real time and 2 s after the stored state timestamp: 10 of 10. `explorer.exe`
  wrote the timestamp of each of their 12 state writes to the state's `Cloud` record 56–71 ms later.
  After each traced `explorer.exe` restart, the new process wrote the state value with a timestamp
  consistent with the same rule, then wrote its timestamp to the state's `Cloud` record several
  seconds later. The probe's 59 state writes did not change the state's `Cloud` record.
- Windows Settings stamps each slider write 2 s after its own previous write or a settings value it
  read ([finding
  15](#15-while-a-fade-writes-faster-than-windows-settings-windows-ignores-the-slider)). In the two
  drags of `slider-stamps` in which the probe only watched, with no other writer, each of the 56
  slider writes was stamped 2 s after the stored timestamp. A slider drag during a lead therefore
  leaves `Cloud` ahead of real time.

### 2. Windows reverts settings writes stamped below `Cloud`

- A write 1 s below `Cloud` is replaced by the last Windows Settings write, with that write's
  timestamp and temperature. A write exactly at `Cloud` stays. Because `Cloud` does not store a
  temperature, each reverting process writes the temperature from its own in-memory copy of the last
  Windows Settings write.
- `explorer.exe` reverts when it observes the below-`Cloud` value. `ShellHost.exe` does too once
  it has started reading the settings ([finding
  11](#11-shellhostexe-reads-the-settings-only-after-quick-settings-opens)). A newer write can replace
  the below-`Cloud` value before either process reads it, as the zero-delay trials below show.
- Each process usually reverts 2–4 ms after the write. Of the 210 writes below `Cloud` in the
  `revert-window` run, each process reverted 195; the other 15 were the 0 ms rounds in the table
  below. The median delay was 3.4 ms for `explorer.exe` and 2.6 ms for `ShellHost.exe`. The revert
  came 42–67 ms after the write for 31 of the 195 writes for `explorer.exe` and for 10 of the 195
  for `ShellHost.exe`.
- A revert overwrites any write made between the write below `Cloud` and the revert. In
  `revert-window`, ordinary refreshes sent after a write below `Cloud` were lost as follows, out of
  15 at each delay:

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
- After the rewind, `explorer.exe` ignored ordinary refreshes stamped at or below the maximum in 62
  of 63 checks. The exception was a check in an early trial: after a refresh below the maximum, the
  observer reported that the screen was not deep red, the tint it keeps if the refresh is ignored.
  The case skipped the rest of that trial.
- A refresh stamped above the maximum applied in 7 of 7 checks. In the state described in finding
  12, writes stamped above every earlier maximum were ignored too.
- Setting `Cloud` and the settings timestamp to real time together still causes a stale-timestamp
  block (`reset-both`, 1 run). Lowering `Cloud` alone does not affect refreshes (`lower-cloud`, 1
  run).

### 4. A write below `Cloud`, which Windows reverts, releases a stale-timestamp block

After a write below `Cloud` and its revert, ordinary refreshes 1,000–3,000 s below the old maximum
applied in 7 of 7 runs of `revert-after-rewind` and `reverts-while-off`. In the first
`reverts-while-off` run the evidence is indirect: switching Night Light on showed the tint of
a refresh 1,025 s below the maximum. Because switching on during a stale-timestamp block shows the
last tint applied ([finding
8](#8-switching-night-light-off-and-on-releases-neither-the-stale-timestamp-block-nor-the-restart-block)),
that refresh had applied. A write below `Cloud`, with its revert, is the only sequence of registry
writes that released a stale-timestamp block in every run. Its costs:

- The display shows the last Windows Settings temperature from the revert until the next refresh.
- A late revert can overwrite writes for up to about 70 ms ([finding 2](#2-windows-reverts-settings-writes-stamped-below-cloud)).
- It lowers the settings timestamp to `Cloud`, not to real time. `Cloud` is ahead of real time if
  Windows Settings wrote during a lead.

### 5. Whether a preview refresh releases a stale-timestamp block depends on when `explorer.exe` reads the settings

- For an ordinary refresh, `explorer.exe` reads only the preview-off value, because preview turns
  off about 0.2 ms after it turns on.
- The following counts include only trials in which the observer first confirmed the stale-timestamp
  block. A 0/0 preview refresh makes its three writes within 1 ms: 0.2–0.7 ms apart in the traces.
  It released the block in 13 of 15 trials, which include 2 trials with no hold in an earlier
  version of the case. All other timings, including holds of 100 ms to 2 s in that earlier version,
  released it in 2 of 24 trials. None of the 10 trials with a 50 ms gap released it. In two
  `held-preview-after-rewind` runs, a preview held for about 1.9 s released it, and a preview held
  for about 2.9 s did not.
- Process Monitor traced 28 trials with a stale-timestamp block, in summaries that record value
  lengths. In the 12 that released the block, `explorer.exe` read a preview-on value and then its
  next value 0.4–0.7 ms later. In the 16 that did not, the next read came about 50 ms later, or
  `explorer.exe` did not read a preview-on value.
- A writer cannot detect which values `explorer.exe` read. A failed attempt does not release the
  block, but it leaves the registry in the same state as a successful one. A writer therefore cannot
  rely on a 0/0 preview refresh to release a stale-timestamp block.

### 6. An `explorer.exe` restart ends a stale-timestamp block but causes a restart block

- A restarted `explorer.exe` applies the stored temperature at startup, even when its timestamp is
  below the old maximum: 5 of 5 runs (`stale-timestamp-survives-restart`,
  `explorer-restart-after-rewind`).
- After a preview refresh or a held preview releases the restart block, ordinary refreshes stamped
  49 s to about 6,000 s below the old maximum applied in 4 of 4 runs.

### 7. Every `explorer.exe` restart blocks ordinary refreshes

- After each restart, `explorer.exe` ignored ordinary refreshes whatever their timestamp: 35 of 35
  checks. After the sign-out in finding 12, ordinary refreshes applied without a restart block.
- Opening the Night Light page in Windows Settings did not release the block (2 runs). A slider
  write released it (3 runs).
- A preview refresh releases the block when preview stays on long enough. Process Monitor traced
  20 trials, in summaries that record value lengths. The block was released in all 15 in which
  `explorer.exe`'s first read of a preview-on value and its read of the preview-off value were at
  least 12.6 ms apart, and in none of the 5 in which the two reads were at most 8.2 ms apart.
- By the probe's own clock, from its preview-on write to its preview-off write, previews kept on
  for 8.6 ms or longer released the block, and previews kept on for 7.0 ms or less did not.
  Keeping preview on for about 9 ms or longer therefore released the block in these measurements.
- Previews held for 1.7 to 4.6 s released the block in 4 of 4 runs. Ordinary refreshes applied
  after each release.
- An ordinary refresh turns preview off about 0.2 ms after turning it on, so it cannot release the
  block.
- No preview timing reliably released both blocks. A 0/50 preview refresh released the restart
  block in 3 of 3 traced trials but a stale-timestamp block in only 1 trial. In the traced trials, a
  stale-timestamp block was released only when `explorer.exe` read the next value within 0.7 ms of
  a preview-on value, and a restart block only when the two reads were more than 8.2 ms apart.

### 8. Switching Night Light off and on releases neither the stale-timestamp block nor the restart block

- During a stale-timestamp block, switching on showed the last tint the display applied, not the
  stored tint, and the next ordinary refresh was ignored (2 runs).
- After a restart, switching on showed the stored tint, and the next ordinary refresh was ignored
  (2 runs).

### 9. Without a block, preview refreshes apply with each of the four tested timings

With neither block in place, preview refreshes with all four timings (0/0, 0/50, 50/0, and 50/50)
applied, and ordinary refreshes after them applied: 8 of 8 trials in 2 runs. A 50/50 preview refresh
therefore applies both without a block and after a restart. For this reason, the probe restores the
original temperature with a 50/50 preview refresh.

### 10. Restarting `ShellHost.exe` does not affect refreshes

An ordinary refresh after a `ShellHost.exe` restart applied in 3 of 3 runs.

### 11. `ShellHost.exe` reads the settings only after Quick Settings opens

A newly started `ShellHost.exe` did not read the settings or revert writes until Quick Settings
opened. From then on, it read every change and reverted writes below `Cloud`, even after Quick
Settings closed. This held in the 5 traces that show `ShellHost.exe` start reading the settings,
including 2 in which the observer opened Windows Settings through Quick Settings. Windows also ended
and relaunched `ShellHost.exe` twice during runs. Neither new process read the settings. For the
first relaunch, the Windows Application log shows no crash.

### 12. Signing out restored updates before process restarts were tested

The first version of this probe ran four times on 2026-10-18. Each run made a sweep and later
rewound. A sweep was 257 ordinary refreshes at up to 144 Hz that stepped the temperature from
6500 K down to 2400 K and back up, about 30 K per refresh. Only run 1 made a second sweep after its
rewind.

In the third run, the display already ignored the first sweep,
and from then on it ignored every settings write. This included writes stamped above every earlier
maximum. When the observer used the Strength slider in Windows Settings, pressing the slider, which
turns preview on, changed the display, but dragging and releasing it did not. Signing out and back
in restored updates. Restarting `explorer.exe` or `ShellHost.exe` had not been considered or tried
at that point, so this run does not establish that signing out was necessary. The Windows
Application log shows no `explorer.exe` crash in that period.

Later tests showed that restarting `explorer.exe` clears the stale-timestamp block, followed by a
preview to release the restart block (finding 6). This supports an in-memory state explanation,
but the earlier state was not reproduced to test either process restart against it.

The second run, the last one before that state, had these
conditions:

- A sweep.
- Two writes below `Cloud` that Windows reverted, the second with Night Light off.
- After each revert, the probe wrote back the settings and on/off state from the start of the run.
  `Cloud` was about 310 s ahead of real time at the reverts, and each revert set the settings
  timestamp to `Cloud`, so the write after the first revert and the write after the second revert
  had the same timestamp.
- The third run started 12 minutes after the second ended.

Four replays did not reproduce the state:

- Two `reverts-while-off` runs repeated a burst and the two reverts, the second with Night Light
  off.
- Two `reverts-while-off --cloud-ahead` runs also moved `Cloud` 134 s and 439 s ahead of real time,
  so the refresh after the first revert and the refresh after the second revert had the same
  timestamp. They waited up to about 7 minutes before switching on.

None of the replays made a sweep or repeated the exact writes of the first probe version.

### 13. Fades with preview held on look gradual at 30 to 480 writes per second, including after a restart

- In `drag-fade`, held drags of 1 s and 30 s at 480 writes per second looked gradual and ended on
  red in 4 of 4 fades, including right after an `explorer.exe` restart. Per-step drags 3 s apart
  changed the screen in visible steps and ended on red in 2 of 2 fades, including after a restart.
- In `drag-rate`, 1-second held drags from neutral to red looked gradual at 30, 60, 144, 240, and
  480 writes per second: 10 of 10 answers, in 2 rounds of random order, without the observer knowing
  the rate. The case did not test rates below 30.
- The probe kept to its schedule. The 1-second drag at 480 writes per second made 480 writes with a
  median gap of 2.0 ms and a largest gap of 2.8 ms; the 30-second drag made 14,400 writes with a
  largest gap of 3.1 ms.
- `explorer.exe` read 29,855 of the 29,868 writes of `drag-fade`, with a median delay of 0.1 ms,
  and never missed two writes in a row.

### 14. While preview stays on, writes apply with Night Light off and after a restart

In `drag-hold`, preview stayed on for 123 s. Each of 5 writes with preview on applied: at the
start, after the observer switched Night Light off, with the Night Light page of Windows Settings
open, after an `explorer.exe` restart, and after 2 minutes. When the probe then turned preview off,
with Night Light still off, the screen returned to no tint.

### 15. While a fade writes faster than Windows Settings, Windows ignores the slider

Most slider writes had timestamps at or below the highest earlier write. Visual observations showed the slider's temperature only briefly; the remaining runs support a timestamp-based
inference rather than a direct observation of the display.

#### Runs and timestamp comparison

`drag-interference` ran four times. Each time, the probe made a held drag while the observer dragged
the Strength slider in Windows Settings: at 30 writes per second in the first three runs and at 240
in the fourth. The first two runs used the earlier library build that stamped
`max(now, previous + 2)`, and the last two used the current rule. `restart-after-slider` ran three
times with the current rule, at 240, 144, and 60 writes per second. `slider-stamps` and two `watch`
captures also traced slider writes, with and without probe writes. Below, the highest earlier
timestamp is the highest settings timestamp written earlier in the trace, which stands in for the
maximum:

#### How Windows Settings chose timestamps

- Windows Settings stamps each slider write 2 s after its own previous write or a settings value it
  read. In the traces of these runs, 512 of 522 slider writes were stamped 2 s after Windows
  Settings' previous write or after a value it read up to 191 ms before the write. The other 10 were
  stamped 2 s after a probe write stored 62–180 ms earlier, so the trace does not show how Windows
  Settings got that value.
- It does not reread the stored value right before each write. While `slider-stamps` wrote the
  stored settings back unchanged once per second during a drag, the next slider write after 7 of the
  10 probe writes came 5–55 ms later and had the same timestamp as the probe write, because both
  were stamped 2 s after Windows Settings' previous slider write. After the other 3, the next slider
  write came 175–200 ms later and was stamped 2 s after the probe write.

#### Slider and fade writes during interference

- Windows Settings wrote at most about once every 60 ms, and the probe every 33 ms or more often, so
  the stored timestamp stayed ahead of the slider writes. In the seven fades of `drag-interference`
  and `restart-after-slider`, 273 of 286 slider writes were stamped at or below the highest earlier
  timestamp. All 13 others came in the first `drag-interference` run, in which the observer pressed
  the slider several times: 9 came after the probe stopped writing, and the other 4 each came right
  after a press. In the first two runs, the observer saw the fade continue, and the screen rarely
  showed the slider's temperature, each time for a few frames; the later runs did not record what
  the screen showed during the fade.
- A slider write's timestamp can come from a read made well before the write: a write stamped from
  Windows Settings' own previous write takes its timestamp from the same read as that previous
  write. During the seven fades, a slider write's timestamp came from a read made up to 1,456 ms
  earlier at 30 writes per second, 552 ms at 60, and 191 ms at 240. At 144, it came from a value
  stored up to 208 ms earlier. The trace does not show when Windows Settings read that value, and
  the longest delay from a traced read at 144 was 77 ms. Meanwhile, the probe's timestamps rose
  above the slider's: with the current rule, slider writes were stamped up to 24 s below the highest
  earlier timestamp at 30 writes per second, 52 s at 60 and at 144, and 90 s at 240. At 30 writes
  per second, the largest gap came at the end of four consecutive slider writes that Windows
  Settings stamped from its own previous write, 581 ms after the read that the series started from.
- In the first two runs, a slider write stored a timestamp up to 8 s below the highest earlier
  timestamp. Each probe write after it was stamped 2 s higher than the stored timestamp, so the next
  one to four probe writes were also stamped at or below the highest earlier timestamp: 217 probe
  writes in the two runs. Windows presumably ignored these writes too; the probe writes after them
  were stamped above the highest earlier timestamp, and the fade continued. In the last two
  `drag-interference` runs, 0 of 8,122 probe writes were stamped at or below the highest earlier
  timestamp.

#### New handles after slider release

- In each `restart-after-slider` run, the probe stopped writing at the observer's release, which
  left the stored timestamp 70 s, 14 s, and 6 s below the highest earlier timestamp at 240, 144, and
  60 writes per second. A per-step drag to red through a new handle, stamped 2 s after the stored
  timestamp, did not change the screen in 3 of 3 runs. A per-step drag to yellow through a new
  handle with the startup margin, stamped 699 s after the stored timestamp, changed the screen in 3
  of 3.

#### Preview state on release

- In the first run, Windows Settings wrote preview off 31 times as the observer pressed and
  released the slider. 25 probe writes were made while preview was off, and each of them turned
  preview back on.
- In the second run, the observer pressed, dragged, and released the slider during an ongoing fade.
  All traced Windows Settings writes had preview on. The trace recorded no preview-off write,
  failed or successful, and the probe never read preview as off.
- Every later traced release turned preview off at the release: 13 of 13, 5 of them during a probe
  fade. In each of the last two `drag-interference` runs, one probe write found preview off and
  turned it back on.
- In the last three `drag-interference` runs, a per-step drag to yellow after the fade applied. In
  the first run, the observer was still dragging at that point. The 9 slider writes made after the
  probe stopped writing were stamped above the stored timestamp, and the screen showed the slider's
  temperature.

With competing writes stopped, clicking the native Strength slider or dragging and releasing it
again starts a fresh preview-on/off cycle. Use that interaction to clear leftover preview manually.
The release observed during the fade does not establish a persistent failure of the native slider.

#### Hypothesis: slider release raced with fade writes

Releasing the mouse is a one-time event, while the probe keeps writing fade updates. A plausible
explanation is that the release's preview-off update used an older timestamp and lost a race with
a newer fade update. If that release update is dropped, the ongoing fade keeps preview on until
another action clears it.

The trace does not establish whether the release produced an update internally or where such an
update was dropped. If Windows Settings wrote it and `explorer.exe` ignored its older timestamp, the
write should still appear in the registry trace. The trace recorded no preview-off write, so a
race before the registry write is plausible. Internal mouse-up handling was not traced.

### 16. Writes without preview apply, except after a restart or during a stale-timestamp block

Each `plain-write` trial set up a condition, then made one plain write. Before each trial during a
stale-timestamp block, the observer confirmed that the block held.

| Condition before the plain write           | Applied  |
| :----------------------------------------- | :------- |
| Normal                                     | 10 of 10 |
| After switching Night Light off and on     | 10 of 10 |
| After an `explorer.exe` restart            | 0 of 10  |
| During a stale-timestamp block             | 0 of 10  |

A fade of 20 plain writes, 3 s apart, changed the screen in visible steps and ended on red.
`explorer.exe` read each write within 0.5 ms.

In `write-while-off`, switching Night Light on after a plain write made while it was off showed
the stored temperature: 5 of 5 trials.

## What this means for programs that write Night Light settings

- **Resetting a timestamp lead:** a rewind alone causes a stale-timestamp block that lasts until a
  refresh is stamped above the old maximum. For a program whose timestamps follow real time, a lead
  of hours means hours of ignored refreshes. A write below `Cloud` cleared the maximum in every run
  but has the costs in [finding
  4](#4-a-write-below-cloud-which-windows-reverts-releases-a-stale-timestamp-block). A rewind followed
  by a 0/0 preview refresh releases the stale-timestamp block only when `explorer.exe` reads the next
  value within about 0.7 ms of a preview-on value (finding 5).
- **Any program that writes the settings:** stamp each write above both the stored timestamp and
  the highest timestamp the program has written, as `max(now, previous + 2, highest + 2)` does in
  `win-nightlight-lib`. After writing below `Cloud`, wait before the next settings write to avoid
  a delayed revert overwriting it. A 70 ms wait covers the observed revert delays, but the probe
  did not establish a safe bound under other system loads.
- **After an `explorer.exe` restart:** ordinary refreshes and plain writes are ignored until a
  slider write or a preview that stays on for about 9 ms or longer (finding 7). A 50/50 preview
  refresh applies both without a block and after a restart, at the cost of a third settings write
  and about 100 ms per refresh. A per-step drag also applies after a restart, with two writes.
- **Changing the temperature gradually:** version 0.5.0 of `win-nightlight-lib` removes
  `set_color_temperature(t, RefreshPolicy::Always)`, which made an ordinary refresh in one call.
  `apply_color_temperature` makes a per-step drag while Night Light is on, `set_color_temperature`
  makes a plain write, and `start_preview`, `preview_color_temperature`, and `end_preview` make a
  held drag. The [color temperature transitions guide](../../docs/color-temperature-transitions.md)
  describes which to use for one-off changes, ramps, and fades.

## Cases

Run a case with `just <case> [arguments]`. Each case that asks about the screen first makes a
neutral and then a red ordinary refresh, asks about each, and stops if the display does not follow
them. The table shows which findings and suite steps each case covers. Expand the procedures below
for arguments, defaults, and the sequence each case runs.

| Case | Findings | Suite part.step |
| :-- | :-- | :-- |
| `revert` | 2 | 1.2 |
| `cloud-boundary` | 2 | 1.3 |
| `revert-window [passes]` | 2 | 1.4 |
| `stale-timestamp [burst]` | 3 | 2.1 |
| `revert-after-rewind [burst]` | 3, 4 | 1.6 |
| `held-preview-after-rewind [burst]` | 5 | 2.2 |
| `preview-timing <blocker> [rounds]` | 5, 7, 9 | 2.4, 2.5, 3.1, 3.6 |
| `switch-release <blocker>` | 8 | 2.3, 3.5 |
| `stale-timestamp-survives-restart` | 6 | 3.4 |
| `explorer-restart-after-rewind [burst]` | 6, 7 | 4.1 |
| `held-preview-after-explorer-restart` | 7 | 4.2 |
| `restart-explorer` | 7 | 4.5 |
| `restart-shellhost` | 10 | 3.3 |
| `quick-settings-reader` | 2, 11 | 3.2 |
| `lower-cloud` | 3 | 4.3 |
| `reset-both` | 3 | 4.4 |
| `reverts-while-off [burst]` | 4, 12 | 1.5 |
| `reverts-while-off [burst] --cloud-ahead` | 4, 12 | 1.1 |
| `drag-fade [--rate <hz>]` | 13 | 5.1 |
| `drag-rate [rounds]` | 13 | 5.2 |
| `slider-stamps` | 1, 15 | 5.3 |
| `drag-interference [--rate <hz>]` | 15 | 5.4 |
| `restart-after-slider [--rate <hz>]` | 15 | 5.5 |
| `drag-hold` | 14 | 5.6 |
| `plain-write [trials]` | 16 | 6.1 |
| `write-while-off [trials]` | 16 | 6.2 |
| `watch [seconds]` | — | — |

<details>
<summary>Case procedures, arguments, and defaults</summary>

### `revert`

Writes the settings 1 s below `Cloud` and records every change to the settings and `Cloud` values in the next second.

### `cloud-boundary`

Writes the settings at `Cloud`, then 1 s below it, and records whether each write stays.

### `revert-window [passes]`

Runs 14 delays per pass, 15 passes by default: writes below `Cloud`, waits 0–120 ms, makes an ordinary refresh, and checks whether a revert overwrote it.

### `stale-timestamp [burst]`

Makes a burst, rewinds to the later of real time and `Cloud`, then refreshes below the maximum and above it.

### `revert-after-rewind [burst]`

Causes a stale-timestamp block as `stale-timestamp` does, writes below `Cloud`, then refreshes below the maximum.

### `held-preview-after-rewind [burst]`

Causes a stale-timestamp block, holds a preview until the observer answers, then refreshes below the maximum.

### `preview-timing <blocker> [rounds]`

Tries the 0/0, 0/50, 50/0, and 50/50 timings in each of `rounds` rounds, 2 by default.

Each trial sets up the blocker (`none`, `stale-timestamp`, or `explorer-restart`), checks the screen, makes the preview refresh, then an ordinary refresh.

`--timing <gap>/<hold>` tries only that timing.

### `switch-release <blocker>`

Blocks refreshes, switches Night Light off and on, then refreshes.

### `stale-timestamp-survives-restart`

Causes a stale-timestamp block, restarts `explorer.exe`, makes a 50/50 preview refresh, then refreshes below the old maximum.

### `explorer-restart-after-rewind [burst]`

Causes a stale-timestamp block, restarts `explorer.exe`, refreshes, holds a preview, then refreshes below the old maximum.

### `held-preview-after-explorer-restart`

Restarts `explorer.exe`, refreshes, holds a preview, then refreshes.

### `restart-explorer`

Restarts `explorer.exe` and refreshes.

If that refresh is ignored, asks the observer to open the Night Light page and refreshes; if that is ignored too, asks for a slider write and refreshes.

### `restart-shellhost`

Restarts `ShellHost.exe` and refreshes.

### `quick-settings-reader`

Restarts `ShellHost.exe`, refreshes and writes below `Cloud`, asks the observer to open and close Quick Settings, then refreshes and writes below `Cloud` again.

Run it under `just trace`: the trace summary shows when `ShellHost.exe` starts reading the settings.

### `lower-cloud`

Lowers `Cloud` by 600 s and refreshes.

### `reset-both`

Makes a burst of 64 refreshes, sets `Cloud` and the settings timestamp to real time, then refreshes below the maximum and above it.

### `reverts-while-off [burst]`

Makes a burst, then twice writes below `Cloud`, refreshes, and switches Night Light off.

Switches it on and refreshes below and above the maximum.

If the refresh above the maximum is ignored, tries these releases in order until one works: a 50/50 preview refresh, switching Night Light off and on, a slider write, and a restart followed by a preview refresh.

### `reverts-while-off [burst] --cloud-ahead`

Adds a slider write before the burst, which moves `Cloud` ahead of real time, and waits for real time to pass the stored timestamp before switching on.

### `drag-fade [--rate <hz>]`

Fades from neutral to red with a 1-second and a 30-second held drag at `rate` writes per second, by default the refresh rate of the fastest active display, and with 20 per-step drags 3 s apart.

Makes the three fades, then each again after an `explorer.exe` restart.

After each fade, asks how the screen changed and whether it ended on red.

### `drag-rate [rounds]`

Makes 1-second held drags from neutral to red at 30, 60, 144, and 240 writes per second and at the fastest display's refresh rate, leaving out rates above it.

Runs 2 rounds by default, each in a new random order.

Asks how each fade looked without naming its rate.

### `slider-stamps`

Runs these steps:

1. Watches the observer press, drag, and release the slider in Windows Settings twice.
2. Writes the stored settings back unchanged once per second for 10 s during another drag.
3. Watches the observer switch Night Light off and on in Quick Settings and Windows Settings.

Run it under `just trace`.

### `drag-interference [--rate <hz>]`

Makes a 30-second held drag at `rate` writes per second, 30 by default, that fades between neutral and red while the observer drags and releases the slider in Windows Settings once.

Then makes a per-step drag to yellow and asks whether it applied.

### `restart-after-slider [--rate <hz>]`

Makes a held drag at `rate` writes per second, 240 by default, that fades near neutral while the observer drags and releases the slider in Windows Settings once.

Stops writing at the release, then makes a per-step drag to red through a new handle and one to yellow through a new handle with the startup margin, and asks after each which tint shows.

### `drag-hold`

Turns preview on and keeps it on for at least 2 minutes.

Writes a tint with preview on and asks whether it applied at these points:

- At the start.
- After the observer switches Night Light off.
- After the observer opens the Night Light page.
- After an `explorer.exe` restart.
- After 2 minutes.

Then turns preview off.

### `plain-write [trials]`

Fades from neutral to red with 20 plain writes 3 s apart and asks how the screen changed.

Then makes `trials` plain writes, 10 by default, in each of four conditions: normal (no block), after switching Night Light off and on, after an `explorer.exe` restart, and during a stale-timestamp block.

### `write-while-off [trials]`

Runs these steps:

1. Switches Night Light off.
2. Makes a plain write of a tint other than the one on screen.
3. Switches Night Light on and asks which tint the screen shows.

Repeats this `trials` times, 5 by default.

### `watch [seconds]`

Prints each change to the settings and `Cloud` timestamps without writing.

</details>

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
  preview is on. Either can lead to a settings write during the case: Windows Settings writes when
  its slider moves, and the slider or program that turned preview on writes again to turn it off.
- Stop other programs that write Night Light settings, such as scripts that run `wnl`, this
  repository's command-line app, in a loop. The probe does not detect them.

### Commands

```bash
just <case> [arguments]               # run one case
just trace <seconds> <case> [args]    # run one case under a Process Monitor capture
just trace 3600 suite <part>          # run one suite part, 1 to 6, under one capture
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

After a fade, the probe also asks how the screen changed. Type:

- `1` if it changed gradually without visible steps.
- `2` if it changed gradually in visible steps.
- `3` if it changed all at once.
- `4` if it did not change.
- `0` for anything else or when unsure.

Before a case that makes many writes, the probe prints how far they can move the settings timestamp
ahead of real time and waits for Enter.

### Run every case with the suite

`suite <part>` runs one of the six parts that together run every case except `watch`, and writes one
results file per case. Parts 2 to 4 take about 2 minutes each. Part 1 takes longer because of the
wait below. Part 5 takes about 17 minutes and part 6 about 5 minutes. The order keeps cases from
changing each other's conditions:

- The `--cloud-ahead` replay runs first, while the settings timestamp is close to real time,
  because it waits for real time to pass that timestamp. It prints how long it will wait.
- Cases that write `Cloud` or ask for a slider write change the timestamp that reverts restore.
  Apart from the `--cloud-ahead` replay and the fallback releases of `reverts-while-off`, these
  cases therefore run in parts 4 and 5, after every case that writes below `Cloud`.
- The drag cases in part 5 run after parts 1 to 4, because they move the settings timestamp hours
  ahead of real time. If part 1 runs after part 5, its first case waits until real time passes
  that timestamp.

A part stops when a step fails, for example when the display does not follow the first neutral and
red refreshes, or when the observer types `quit`. It then prints the `--from` step to resume at.

### Changes a run leaves behind

- Each case changes the temperature and the on/off state, then restores both. Restoring writes a
  settings timestamp above every timestamp the case saw, so a case with a 256-refresh burst leaves
  the lead about 1,000 s larger, less any time it waits. Parts 1 to 4 leave a lead of about 6,000 s.
- Each write of a held drag advances the settings timestamp by 2 s. On a 480 Hz display, `drag-fade`
  leaves a lead of about 17 hours and `drag-rate` about 1 hour. `drag-interference` writes 30 times
  per second by default on any display and leaves about 30 minutes; with `--rate 240`, it leaves
  about 4 hours. `restart-after-slider` writes 240 times per second by default and leaves up to
  about 4 hours, plus up to 699 s from the startup margin. Windows still applies each later library
  write, because each is stamped above the one before. The lead shrinks only as real time passes.
- A slider write during a lead leaves `Cloud` ahead of real time by that lead.
- Cases that restart `explorer.exe` hide the taskbar for a few seconds.
- `revert-window` makes the screen flicker for about 2 minutes.
- Killing the probe, for example with Ctrl+C, skips restoring. If Night Light then ignores
  temperature changes, restart `explorer.exe` and move the Strength slider in Windows Settings, or
  sign out and back in.

## Results and traces

Each case writes `results/<case>-<unix seconds>.jsonl`, one JSON record per line:

- `environment`: the Windows build, GPUs, display refresh rates, Git revision and status of the
  probe, and `rustc --version`.
- Snapshots, such as `start`, `burst`, and `restored`: the settings and `Cloud` timestamps, the
  lead, the temperature, the preview flag, and the `explorer.exe` and `ShellHost.exe` process IDs.
- `observer`: the question's label, the tint the question checks for, and the observer's answer. A
  question about a refresh checks for that refresh's tint. A question about a block checks for the
  tint that stays on screen if the block holds. An answer about a fade has `question: smoothness`
  and one of `gradual`, `stepped`, `at_once`, `unchanged`, or `other`.
- `held_drag`: for each held drag, the planned rate and duration, the number of steps, the median,
  99th percentile, and largest time between steps, and the numbers of the steps, counted from 1, at
  which preview was off before the step's write.
- Records specific to a case, such as `trial_outcome`, `fade_outcome`, or `plain_outcome`.

`just trace` writes `traces/trace-<unix seconds>.pml` and its summary `.tsv`. Captures reach about
90 MB. Git ignores both `results/` and `traces/`. Each summary line has, separated by tabs:

1. Milliseconds since the first event.
2. Process name.
3. Process ID.
4. `read` or `write`.
5. The registry location: `Current`, for the values Night Light uses, or `Cloud`, for the `Cloud`
   record; and `settings` or `state`, for the settings or state value.
6. The timestamp in the settings or state value. Summaries written by earlier versions of
   `summarize-trace` leave it empty for state values; rerun `summarize-trace` on the `.pml` capture
   to add it.
7. The value length in bytes. Process Monitor records only the first 16 bytes of a value, so the
   length is the only sign of the preview flag: a value with preview on is 3 bytes longer.
8. The UTC time of the event.

Four commands of the `trace/` project read the summaries:

```bash
# What explorer.exe read during each preview refresh (findings 5 and 7)
uv run --locked --project trace preview-reads results/preview-timing-<n>.jsonl traces/trace-<m>.tsv
# Which processes reverted each write below Cloud, and how fast (finding 2)
uv run --locked --project trace reverts traces/trace-<m>.tsv [results/<case>-<n>.jsonl]
# Whether explorer.exe read each probe write, and how soon (drag and plain-write cases)
uv run --locked --project trace write-reads traces/trace-<m>.tsv [results/<case>-<n>.jsonl]
# How each process stamped its writes, and what each slider write was stamped from (findings 1, 15)
uv run --locked --project trace stamps traces/trace-<m>.tsv [results/<case>-<n>.jsonl]
```

`trace/` is a uv project with Python 3.15. `just trace-check` runs its lint, type-check, test,
and dependency audit tasks, which CI also runs. `just trace-fix` applies lint fixes and formatting.

### Suite run on 2026-10-19

Parts 1 to 4 ran on 2026-10-19 from 02:10 to 02:28 UTC. Of 134 answers, 115 named the tint the
question checked for. Each of the 19 others came from a step during a stale-timestamp block or a
restart block and named the tint the display had last applied. These questions asked whether a
refresh or a held preview applied, or which tint switching Night Light on showed.

Parts 5 and 6 ran from 06:57 to 07:21 UTC, with a second `drag-interference` run at 07:10. Of 93
answers about a tint, 72 named the tint the question checked for:

- 20 of the others answered plain-write trials after a restart or during a stale-timestamp block,
  and each named the tint already on screen, which means the write was ignored.
- The last one answered whether the per-step drag to yellow in the first `drag-interference` run
  applied. The observer was still dragging the slider at that point.

The observer also gave 17 answers about how a fade looked. `write-while-off`, added to part 6
afterward, ran on its own at 07:57 UTC, and its 5 answers named the stored tint.

`slider-stamps` and two more `drag-interference` runs, at 30 and 240 writes per second, ran on their
own from 18:35 to 18:59 UTC with the current library. Each of their 6 answers named the tint the
question checked for.

`restart-after-slider` at 240, 144, and 60 writes per second and two `watch` captures ran from 20:04
to 20:16 UTC. Of the 12 answers in the `restart-after-slider` runs, 9 named the tint the question
checked for. Each of the other 3 answered the question after the per-step drag through a new handle
without the startup margin, and named the tint already on screen, as finding 15 describes.

### Recheck the findings

Run suite parts 1 to 6 under `just trace`, as [Commands](#commands) shows. Then, for each case,
compare its results file and the trace summary with the findings that [Cases](#cases) lists for it.
`reverts` prints the trace measurements for finding 2, `preview-reads` for findings 5 and 7,
`write-reads` for findings 13 and 16, and `stamps` for findings 1 and 15. Results can differ on
another Windows build, display, or observer; the `environment` record at the start of each results
file names the build and display.

### Runs before the suite

- 2026-10-18 from 21:46 to 22:41 UTC: the four runs of the first version of this probe, described in
  finding 12. Its code is no longer in the repository. In run 1, the observer reported that the
  second sweep, stamped below the first sweep's maximum, changed the display. The later cases write
  tints that differ clearly from each other and show that such refreshes are ignored (finding 3).
  That report is therefore treated as misjudged. Run 3 already showed the state in finding 12, and
  run 4 followed the sign-out.
- 2026-10-18 23:33 UTC to 2026-10-19 01:58 UTC: exploratory runs of the current cases and their
  earlier versions. Their questions named the tint they checked for and took yes or no. Four of
  these runs came from an earlier version of `preview-timing` that varied only the hold. Five other
  runs ended before their first measurement.

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
    observed during a stale-timestamp block was stamped above it. Slider writes stamped at or below
    the highest timestamp during a fade were ignored (finding 15).
  - Held drags at fewer than 30 writes per second.
  - What the screen shows during a slider drag while a program writes seconds apart, as a ramp does.
  - Whether `explorer.exe` ignores state writes stamped at or below the highest state timestamp it
    has applied, and whether Windows reverts state writes stamped below the state's `Cloud`
    timestamp.
  - Preview timings between 1 and 50 ms during a stale-timestamp block.
  - Whether an `explorer.exe` that Windows restarts after a crash blocks refreshes as a restart by
    the probe does.
  - Whether system load changes the revert delays.
- The trace does not establish whether mouse-up produced an update internally or where such an
  update was dropped during a concurrent fade (finding 15). A race with newer fade updates is plausible. Applications should finish with
  `end_preview`; users can clear preview by stopping other writers and using the native slider again.
- Process Monitor deleted some early capture files, for an unknown reason. The trace recipe renames
  each capture after it ends, and no renamed capture has been lost.
