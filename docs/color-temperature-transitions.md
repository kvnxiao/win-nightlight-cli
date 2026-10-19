# Color Temperature Transitions

Use `apply_color_temperature` for a single change, `set_color_temperature` for a long ramp, and
the preview methods for a short fade. This guide explains how to choose the steps and handle
interference from the Strength slider in Windows Settings.

These recommendations draw on measurements from the [timestamp rewind probe](../probes/timestamp-rewind/README.md)
on Windows 11 build 26300, with a 480 Hz display. Windows does not document this behavior. In this guide, a **fade** lasts up to 30 s and a **ramp** lasts
longer; that cutoff is a recommendation, not a Windows limit.

## Why a successful write may not change the screen

You might expect writing a new temperature to the registry to update the screen. With Night Light,
a write can succeed while the display stays the same. This is frustrating to work with, especially
when another app or the Windows slider is changing the same settings.

The timestamp is part of the problem. It has whole-second precision, even though a fade can write
many times per second. `explorer.exe` behaves as if it remembers the highest timestamp it has applied
and ignores older or equal timestamps. `explorer.exe` and `ShellHost.exe` also appear to keep in-memory copies
of Windows settings and can restore them based on timestamp comparisons. `ShellHost.exe` starts reading
these settings after Quick Settings opens.

That makes the timestamp more than a record of when you wrote a value. Rapid writes advance it
ahead of real time. Moving it backward can leave your app writing values that Windows ignores or
overwrites. Incorrect timestamp management can therefore cause bugs that look like failed
temperature changes, even when the registry calls succeed.

The registry format uses reverse-engineered schemas. How Windows components use those values was
evaluated empirically through controlled registry writes, Process Monitor traces, and watching the
display, without decompiling or disassembling Windows. The in-memory state is inferred from those
observations. The rest of this guide explains how to work with that behavior.

## Choose a method

| Change | Method | Behavior |
| :-- | :-- | :-- |
| One new temperature | `apply_color_temperature(t)` | Briefly previews the temperature when Night Light is on; stores it when off. |
| Each step of a ramp | `set_color_temperature(t)` | Stores the temperature without changing preview; skips an unchanged temperature. |
| Start a fade | `start_preview()` | Turns preview on, keeping the temperature. |
| Each step of a fade | `preview_color_temperature(t)` | Stores the temperature with preview on. |
| Finish a fade | `end_preview()` | Turns preview off, keeping the temperature. |

**Preview** makes Windows show the stored temperature even while Night Light is off. The slider
normally turns it on when pressed and off when released. A **brief preview** lasts 50 ms; a
**held preview** lasts from `start_preview` to `end_preview`.

The library reads and writes the Night Light **settings** registry value, which stores the
temperature, schedule, and preview flag. A separate **state** value records whether Night Light is
on. Here, a **write** means one settings write, and a **handle** means one `Nightlight` value.

### Waits, return values, and errors

`apply_color_temperature` reads both settings and state:

- **Night Light on:** writes the temperature with preview on, waits 50 ms, then rereads the settings
  and turns preview off if needed. It does this even when the temperature already matches. The wait
  blocks the calling thread; use a blocking thread in async code.
- **Night Light off:** stores the temperature with preview off, without waiting. It skips the write
  only when the temperature already matches and preview is already off.
- **Cleanup failure:** tries to turn preview off up to three times, without waiting between attempts.
  If all fail, it returns `ErrorKind::PreviewCleanup`. Preview may remain on; retrying the call
  repeats the whole sequence.

On success, `apply_color_temperature` ends any existing preview and returns whether preview was on
before the call. Each preview method also returns the previous preview flag. They
always write once, do not sleep, and do not read the on/off state.

`end_preview` makes one attempt. If it fails, preview may remain on; retry cleanup. A successful
registry write does not confirm that the display changed. Avoid overlapping calls: another writer
can change the settings between a method's read and write.

## One handle per application, with a startup margin

Create one handle with `Nightlight::new().with_startup_margin()` and keep it for the application's
lifetime. Use an `Arc` to share it across threads. Reusing the handle preserves its record of the
highest timestamp it has written to settings and state.

Each settings write has a timestamp in whole seconds. The probe found that `explorer.exe`, which
applies Night Light to the display, behaves as if it remembers the highest timestamp it has applied.
It usually ignores writes at or below that maximum. The library stamps each write at least 2 s
above both the stored timestamp and the handle's own highest timestamp, or at the current time if
that is later.

During a fade, the slider can overwrite the stored timestamp with a lower one. The existing handle
can still write above its own maximum, but a new handle has no record of it. The registry does not
expose `explorer.exe`'s remembered maximum.

The startup margin helps with that case:

- It stamps the handle's first successful settings write at least 699 s after the timestamp read
  from settings, instead of the usual 2 s.
- In the tested restart scenarios, a new handle's brief preview was ignored without the margin
  and applied with it. The slider had lowered the stored timestamp below the highest earlier write.
- **699 s is an estimate, not a guaranteed bound.** It adds up to about 12 minutes to the timestamp's
  lead over real time each time a new handle first writes settings.
- The margin does not apply to state writes. `wnl` does not enable it, because each command creates
  a new handle and would add the margin again.

### How the margin was estimated

Windows Settings can reuse a timestamp from an earlier read across several slider writes. During
that delay, a fast fade can advance timestamps well beyond the slider's next write.

The estimate combines the longest measured delay, 1.456 s, with this guide's suggested cap of 240
writes per second: `2 s/write × 240 writes/s × 1.456 s = 698.88 s`, rounded up to 699 s. That combination was not itself a
measured worst case. See [probe finding
15](../probes/timestamp-rewind/README.md#15-while-a-fade-writes-faster-than-windows-settings-windows-ignores-the-slider)
for the runs and trace limitations.

## One-off changes

Call `apply_color_temperature`. This is also what `wnl temp` calls.

After the probe forcibly restarted `explorer.exe`, temperature writes without preview were ignored.
This guide calls that a **restart block**. Keeping preview on for about 9 ms or longer released it
in the measured trials. The library uses a 50 ms brief preview when Night Light is on.

Brief previews spaced several seconds apart changed the screen in visible steps and reached the
final temperature, both normally and after a restart. Recovery after an automatic crash restart was
not tested.

## Ramps

For a change lasting more than 30 s:

1. Make the first step with `apply_color_temperature`.
2. Make subsequent steps with `set_color_temperature`, using small temperature increments.
3. About once a minute, use `apply_color_temperature` for a step instead. This is intended to recover
   from an unnoticed `explorer.exe` restart; it is not a guaranteed recovery deadline.

A 10 K increment is a starting point: a 30-second held preview from 6500 K to 1200 K looked gradual
with those steps. That test used a held preview, so it does not establish how large a ramp step can
be before it looks stepped.

While Night Light is off, `apply_color_temperature` stores the temperature with preview off.
`set_color_temperature` preserves the preview flag, so it does not end a preview already in progress.
A ramp can store temperatures while Night Light is off. In the tested cases,
temperatures written while it was off appeared when it was switched on.

Frequent writes move the settings timestamp ahead of real time. This difference is its **lead**.
For a ramp from 6500 K to 2700 K in 10 K increments:

| Ramp duration | Time between steps | Lead from ordinary steps |
| :-- | :-- | :-- |
| 60 minutes | About 9.5 s | Does not grow |
| 20 minutes | About 3.2 s | Does not grow |
| 60 seconds | About 158 ms | About 12 minutes |

These estimates exclude the startup margin and the extra writes for brief previews. Fewer, larger
steps reduce the lead, but may make the change look stepped.

In the probe, writes without preview applied normally and after switching Night Light off and on,
but were ignored after an `explorer.exe` restart. Writes spaced several seconds apart changed the screen
in visible steps and reached the final temperature.

## Fades

For a change lasting up to 30 s, hold preview on: call `start_preview`, write each step with
`preview_color_temperature`, and finish with `end_preview`. End preview even after a step fails,
because it can keep the screen tinted while Night Light is off.

```rust
use std::thread::sleep;
use std::time::Duration;
use std::time::Instant;
use win_nightlight_lib::ColorTemperature;
use win_nightlight_lib::Nightlight;
use win_nightlight_lib::Result;

fn fade(
    nightlight: &Nightlight,
    temperatures: &[ColorTemperature],
    interval: Duration,
) -> Result<()> {
    nightlight.start_preview()?;
    let stepped = step_through(nightlight, temperatures, interval);
    let ended = nightlight.end_preview();
    stepped?;
    ended?;
    Ok(())
}

fn step_through(
    nightlight: &Nightlight,
    temperatures: &[ColorTemperature],
    interval: Duration,
) -> Result<()> {
    let started = Instant::now();
    let mut previous = None;
    for (step, temperature) in (1..).zip(temperatures) {
        if previous == Some(*temperature) {
            continue;
        }
        if let Some(wait) = (started + interval * step).checked_duration_since(Instant::now()) {
            sleep(wait);
        }
        nightlight.preview_color_temperature(*temperature)?;
        previous = Some(*temperature);
    }
    Ok(())
}
```

The example attempts cleanup even after a step fails. If both the step and cleanup fail, it returns
the step error; an application that needs to report both failures must handle both results.

### Choose the rate and skip repeated temperatures

Write at most once per frame of the fastest active display, with a suggested cap of 240 writes per
second. The application can detect the refresh rate or let the user choose a cap. This limits the
write rate; it does not synchronize writes with frames or guarantee that every write is displayed.

Lower rates build less timestamp lead. In the probe, fades looked gradual across the tested range
of 30–480 writes per second. Rates were presented in random order without telling the observer
which was running. Rates below 30 were not tested. Held previews also looked gradual immediately
after an `explorer.exe` restart.

Skip temperatures that equal the previous step, as the example does. They do not change the screen
but still advance the timestamp. For a 30-second fade from 6500 K to 2700 K in 10 K increments:

| At 240 scheduled steps per second | Settings writes, including preview on/off | Approximate lead added |
| :-- | :-- | :-- |
| Write every step | Thousands | About 4 hours |
| Skip repeated temperatures | Hundreds | About 13 minutes |

These estimates exclude the startup margin.

## Timestamp lead

The usual timestamp step is 2 s per write. Writes more than 2 s apart do not build lead when there
are no other writers. Real time reduces an existing lead at 1 s per second.

| Change | Usual settings writes | Timestamp advance from those writes |
| :-- | :-- | :-- |
| One-off change while Night Light is on | 2 | 4 s |
| One-off change while Night Light is off | 1, or 0 when unchanged | 2 s, or none |
| Ramp step | 1, or 0 when unchanged | 2 s, or none |
| Ramp step with a brief preview | 2 | 4 s |
| Fade with `n` written steps | `n + 2` | `2n + 4` s |
| First settings write with the startup margin | 1 | 699 s instead of the usual 2 s |

The table assumes successful writes, no competing writer, and a stored timestamp at least as recent
as real time. It excludes elapsed time and cleanup retries. An active one-off change can skip its
second write if another writer has already turned preview off. After another writer lowers the
stored timestamp, the handle can jump by more than 2 s to stay above its own earlier writes.

A 1-second fade at 240 writes per second adds about 8 minutes of lead before the startup margin.
That lead alone did not stop later library writes from applying in the probe: their timestamps
continued to increase.

The problem arises when another writer lowers the stored timestamp. `explorer.exe` may keep ignoring
new writes until their timestamps pass its remembered maximum. A handle that wrote that maximum
can pass it on its next write; a new handle may need many writes, or a wait for real time to catch
up. Do not try to remove the lead by moving the timestamp backward yourself. See [probe finding
3](../probes/timestamp-rewind/README.md#3-a-rewind-at-or-above-cloud-stays-in-the-registry-and-causes-a-stale-timestamp-block).

To track the lead, subtract the current time from `Settings::modified`, returned by `read_settings`.
This measures the stored timestamp's lead, not `explorer.exe`'s remembered maximum.

## When the user moves the slider during a gradual change

**During a fade, the application's writes can prevent slider changes from appearing.** Windows
Settings reused older timestamps while the probe wrote more frequently. Nearly all slider writes
were at or below the highest earlier timestamp. In the runs with visual observations,
the observer saw the slider's temperature only briefly. The other runs did not record the display
during the fade, so their display behavior is inferred from timestamps.

The current library keeps each handle's timestamps above its own earlier writes. In the tested
`drag-interference` runs, its writes stayed above the highest earlier timestamp. An earlier build
followed only the stored timestamp; some writes fell at or below that maximum after slider writes. Windows presumably ignored those writes.

**A preview return value is not a slider event.** Releasing the slider usually turns preview off.
The next `preview_color_temperature` call then returns `false` and turns it back on. During a
concurrent fade, a release was observed without a preview-off write. The one-time mouse-up update
may have used an older timestamp and lost a race with ongoing fade updates. The trace does not
show whether the release produced an update internally or where such an update was dropped.
`explorer.exe` ignoring a stored write alone would not explain its absence from the registry trace.

Another program can also turn preview off. The return value cannot reliably identify presses,
drags, or releases.

**To clear preview manually, stop competing writes and use the native slider again.** Click the
Strength slider in Windows Settings, or drag it and release. With no other program writing the
settings, this starts a new preview on mouse-down and turns it off on mouse-up. The concurrent
release observation does not imply that preview remains stuck after subsequent slider interactions.
In the observed sequence after the fade, slider writes advanced the timestamp and changed the screen.

**During a slower ramp, the next step can replace the user's temperature.** Slider writes can apply
between steps. If Windows Settings has not read the latest step, its next write can reuse that
step's timestamp and be ignored. In `slider-stamps`, the probe wrote once per second during a drag:
the next slider write usually had the same timestamp as the probe write, but sometimes advanced it.
The probe did not record the screen during this test; the expected display behavior is an inference.

The application decides whether to continue after a user change. Before a ramp step, compare the
stored temperature with the last one the application wrote. A difference detects a changed value,
not who changed it, and cannot detect a slider move that ends at the same temperature.

## Leftover preview at startup

If an application exits during a held preview, preview can remain on and keep the screen tinted
while Night Light is off. At startup, choose one of these actions:

- To set the application's desired temperature, call `apply_color_temperature`.
- To keep the stored temperature, read the settings and call `end_preview` if
  `Settings::is_previewing` is `true`.

The flag does not identify who started the preview. Ending it can interrupt a slider drag already
in progress. For manual cleanup, stop competing writes, then click the native Strength slider or
drag and release it again to complete a fresh preview-on/off cycle.

## When Windows ignores the application's writes

A successful write with no visible change does not by itself prove `explorer.exe` is ignoring it. Check
whether Night Light is off, whether the temperature changed, and whether another writer replaced
the settings. One possible cause is that `explorer.exe` has already applied a higher timestamp.

These user actions restored updates in the probe:

- **Restart `explorer.exe`.** The new process applied the stored temperature, including after a
  timestamp rewind. Make the next change with `apply_color_temperature` while Night Light is on
  to release the restart block. The probe used `taskkill /f /im explorer.exe`, followed by
  `explorer.exe`; Task Manager's Restart command was not tested. The taskbar disappears briefly
  and open File Explorer windows close.
- **Sign out and back in.** This restored updates in an early run, before restarting `explorer.exe` or
  `ShellHost.exe` had been considered ([finding
  12](../probes/timestamp-rewind/README.md#12-signing-out-restored-updates-before-process-restarts-were-tested)).
  It does not establish that signing out was necessary. After signing back in, even back-to-back
  preview-on/off writes applied.

Suggest these actions to the user when needed. The application should not restart `explorer.exe` itself,
because that interrupts the user's work.
