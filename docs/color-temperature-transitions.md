# Color Temperature Transitions

`win-nightlight-lib` lets an application change the Night Light color temperature at once or
gradually. This guide prescribes which library methods to call for each kind of change and how
often to write the settings. It also describes what happens when the user moves the slider in
Windows Settings during a gradual change.

The measurements come from the [timestamp rewind probe](../probes/timestamp-rewind/README.md), a
program in this repository that writes the Night Light settings in controlled sequences and asks an
observer which tint the screen shows. It ran on Windows 11 build 26300 with one 480 Hz display.

## Terms

- **Settings:** the Night Light settings registry value, not the Windows Settings app. It stores
  the color temperature and the schedule. Whether Night Light is on is stored in a second value,
  the state.
- **Write:** one write of the settings.
- **Slider:** the Strength slider on the Night Light page of Windows Settings.
- **Slider write:** a write that Windows Settings makes while the user presses, drags, or
  releases the slider.
- **Handle:** one `Nightlight` value, which the application creates with `Nightlight::new()`.
- **Block:** a state in which `explorer.exe` ignores temperature changes, such as the restart
  block described below.
- **One-off change:** a change to one new temperature.
- **Fade:** a gradual change that lasts up to 30 s.
- **Ramp:** a gradual change that lasts longer than 30 s.
- **Lead:** how far the settings timestamp, described below, is ahead of real time.

## How Windows applies a temperature

`explorer.exe` reads the settings after each write and applies the temperature to the display.
Whether a write changes the screen depends on three behaviors:

- **Preview:** the settings have a preview flag. While it is on, Windows shows the stored
  temperature, even while Night Light is off. Windows Settings turns preview on while the user
  presses or drags the slider, and off when the user releases it.
- **Timestamp:** each write stores a last-modified time in whole seconds. `explorer.exe` ignores a
  write stamped at or below the highest timestamp it has applied. The library stamps each write 2 s
  after the stored timestamp, the step Windows Settings also uses, or at the current time if that is
  later. Each handle also stamps each write at least 2 s after the highest timestamp it has written,
  so when another writer, such as the slider, stores a lower timestamp, the handle does not stamp
  its next write lower. Apart from the startup margin described below, a write therefore adds at
  most 2 s of lead. Real time removes lead at 1 s per second, so writes 2 s or more apart do not
  make the lead grow.
- **Restart block:** after `explorer.exe` restarts, for example after a crash, it ignores
  temperature changes until a preview stays on for about 9 ms or longer, or until the user moves
  the slider. Switching Night Light on still shows the stored temperature during the block.

## Methods

| Method                         | Writes                                                                                                                                                                   | Use                 |
| :----------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------ |
| `apply_color_temperature(t)`   | While Night Light is on: `t` with preview on, then preview off after 50 ms. While it is off: `t` with preview off, once. No write when `t` is stored and preview is off. | A one-off change    |
| `set_color_temperature(t)`     | `t` once, without changing preview. No write when `t` is already stored.                                                                                                 | Each step of a ramp |
| `start_preview()`              | Preview on, with the temperature unchanged.                                                                                                                              | The start of a fade |
| `preview_color_temperature(t)` | `t` with preview on.                                                                                                                                                     | Each step of a fade |
| `end_preview()`                | Preview off, with the temperature unchanged.                                                                                                                             | The end of a fade   |

This guide calls the preview that `apply_color_temperature` keeps on for 50 ms a brief preview, and
the sequence from `start_preview` to `end_preview` a held preview. A held preview makes the same
writes as a press, drag, and release of the slider.

- `apply_color_temperature` reads the settings and the state before writing.
  - While Night Light is on, it blocks the calling thread for the 50 ms wait, so run it on a
    blocking thread in async code. If turning preview off fails, it tries twice more without
    waiting. If all three attempts fail, it returns an error of kind `ErrorKind::PreviewCleanup`,
    and preview may stay on; calling it again repeats the whole sequence.
  - While Night Light is off, it makes at most one write and does not wait.
  - Either way, it ends any preview that was on and returns whether one was.
- The three preview methods each read the settings, write once even when nothing changes, and return
  whether preview was on before the call. They do not sleep, and they do not read the state. During
  a held preview, the screen shows each temperature as it is written, even while Night Light is off.
- `end_preview` makes one attempt. If its write fails, preview stays on; call it again.

## One handle per application, with a startup margin

Create one `Nightlight` handle when the application starts, and use it for the application's
lifetime. Share it across threads through an `Arc`. Each handle records in memory the highest
timestamp it has written to each value, the settings and the state. A second handle, or a new handle
after the application restarts, starts with no record.

Create the handle with `Nightlight::new().with_startup_margin()`. The handle then stamps its first
settings write at least 699 s, about 12 minutes, after the stored timestamp. The margin is for an
application that restarts, and so creates a new handle, soon after the user dragged the slider
during one of the application's fades. Without the margin, the new handle stamps its first write 2 s
after the lower timestamp that the slider stored, and `explorer.exe` ignores the write. Windows
Settings stamps each slider write 2 s after either its own previous write or a settings value it
read, depending on when it last read the settings. A slider write's timestamp can therefore come
from a read made well before the write, directly or through a series of Windows Settings' own
writes. In the probe's fades at 30, 60, and 240 writes per second, during which the observer dragged
the slider, a slider write's timestamp came from a read made up to 1,456 ms, 552 ms, and 191 ms
earlier. At 144 writes per second, it came from a value stored up to 208 ms earlier, and the trace
does not show when Windows Settings read that value. With the current library, slider writes were
stamped up to 24 s, 52 s, 52 s, and 90 s below the highest timestamp written before them. The
registry does not store the highest timestamp that `explorer.exe` has applied, so a new handle
cannot tell how far below it the stored timestamp is.

During that delay, each write of a fade at this guide's cap of 240 writes per second (see
[Fades](#fades)) raises the highest timestamp by 2 s. The margin is therefore 2 s per write × 240
writes per second × 1,456 ms, rounded up: the longest delay measured at any rate, combined with the
cap. It is an estimate of the worst case, not a bound. The probe tested it at 60, 144, and 240
writes per second by stopping a fade at the moment the user released the slider, as an application
that exits would. Each release left the stored timestamp 6 s to 70 s below the highest timestamp the
probe had written. Each time, a per-step drag through a new handle without the margin did not change
the screen, and one through a new handle with the margin did. The margin adds up to 699 s of lead
once per handle, less than the lead of about 13 minutes that the 30-second fade in [Fades](#fades)
adds when it skips unchanged steps. The `wnl` command-line tool in this repository does not use the margin,
because each `wnl` command creates a new handle, and the margin would add its lead on every command.

## One-off changes

Call `apply_color_temperature`. While Night Light is on, its brief preview applies the temperature
even during a restart block. In the probe, a sequence of 20 brief previews, 3 s apart, changed the
screen in visible steps and ended on its last temperature, both with no block and right after a
restart. The `wnl temp` command in this repository makes this call.

## Ramps

1. Make each step with `set_color_temperature`. Small steps, such as 10 K, make the change look
   gradual: in the probe, a 30-second held preview from 6500 K to 1200 K that changed in 10 K steps
   looked gradual.
2. Make the first step, and one step about every minute after it, with `apply_color_temperature`
   instead. Its brief preview ends a restart block. After an `explorer.exe` restart that the
   application does not notice, the screen keeps the temperature that the new `explorer.exe` applied
   at startup for at most a minute.

While Night Light is off, `apply_color_temperature` only stores the temperature, and
`set_color_temperature` never turns preview on, so a ramp does not need to check whether Night
Light is on. When Night Light turns on, the screen shows the stored temperature: in the probe, 5 of
5 temperatures written while Night Light was off showed when it was switched on.

The lead grows only when steps come less than 2 s apart. A ramp from 6500 K to 2700 K in 10 K steps
has 380 steps: over 60 minutes, one every 9.5 s, and over 20 minutes, one every 3.2 s, so neither
builds lead. Over 60 seconds, the steps come 158 ms apart, and when the ramp ends, the timestamp is
about 12 minutes ahead of real time. A ramp that must build less lead needs fewer, larger steps; the
probe did not test how large a step can be before a ramp looks stepped.

In the probe, writes without preview applied in 10 of 10 trials with no block, and in 10 of 10
after Night Light was switched off and on. A sequence of 20 such writes, 3 s apart, changed the
screen in visible steps and ended on its last temperature. After an `explorer.exe` restart, 0 of
10 applied.

## Fades

Make one held preview: `start_preview`, one `preview_color_temperature` call per step, and
`end_preview`. End the preview even after a failed step, because Windows keeps showing the stored
temperature while preview is on:

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

- **Write rate:** write at most once per frame of the fastest active display, so that each frame
  can show a new temperature. A cap of 240 writes per second is suggested; the application can
  detect the display's refresh rate or let the user set the cap. Lower rates build less lead. In
  the probe, 1-second fades from 6500 K to 1200 K looked gradual at 30, 60, 144, 240, and 480
  writes per second: the observer gave 10 answers, 2 per rate, in random order and without knowing
  the rate. The probe did not test rates below 30.
- **Skip unchanged steps:** skip a write whose temperature equals the previous write's, as the
  example does. Such a write does not change the screen but still adds up to 2 s of lead. A
  30-second fade from 6500 K to 2700 K in 10 K steps at 240 writes per second has 7,200 frames but
  only 380 temperatures after the starting one. Skipping repeats cuts its writes from 7,202 to 382,
  and the lead it adds from about 4 hours to about 13 minutes.

A held preview also applies during a restart block: in the probe, 1-second and 30-second fades
looked gradual right after a restart.

## Timestamp lead

| Change                                  | Writes                 | Lead added, at most |
| :-------------------------------------- | :--------------------- | :------------------ |
| One-off change while Night Light is on  | 2                      | 4 s                 |
| One-off change while Night Light is off | 1, or 0 when unchanged | 2 s, or none        |
| Ramp step                               | 1                      | 2 s                 |
| Ramp step with a brief preview          | 2                      | 4 s                 |
| Fade with `n` written steps             | `n + 2`                | `2n + 4` s          |
| First write of a handle with the margin | 1                      | 699 s               |

A 1-second fade at 240 writes per second adds about 8 minutes of lead. During a lead, Windows still
applies each library write, because each is stamped above the one before. The lead shrinks only as
real time passes. A large lead matters when another program writes the settings stamped with the
current time instead of the stored timestamp. That write moves the timestamp back, and
`explorer.exe` then ignores each write stamped at or below the highest timestamp it has applied
([probe finding
3](../probes/timestamp-rewind/README.md#3-a-rewind-at-or-above-cloud-stays-in-the-registry-and-causes-a-stale-timestamp-block)).
A handle that wrote that highest timestamp stamps its next write above it, so its writes still
apply. A new handle, or another program that stamps 2 s after the stored timestamp, has its writes
ignored until real time or its 2-second steps pass that highest timestamp; for a lead of hours, that
can take hours. For the same reason, never move the timestamp back yourself, for example by writing
the registry value directly.

To see how fast the lead grows, record it after each change: the stored timestamp, which
`read_settings` returns as `Settings::modified`, minus the current time.

## When the user moves the slider during a gradual change

- **During a fade:** Windows Settings writes at most about once every 60 ms during a drag. It stamps
  each slider write 2 s after either its own previous write or a settings value it read, depending
  on when it last read the settings, and it does not reread the settings right before the write. An
  application that writes more often therefore keeps the stored timestamp ahead of the slider's
  writes, and Windows ignores nearly all of them. In seven probe fades at 30 to 240 writes per
  second, 273 of 286 slider writes were stamped at or below the highest timestamp written before
  them. In the first two of those fades, the observer saw the fade continue, and the screen rarely
  showed the slider's temperature, each time for a few frames; the others did not record what the
  screen showed during the fade.
- **Effect on the fade:** most slider writes store a timestamp below the fade's latest write: in the
  probe, up to 90 s below with the current library. The handle stamps the fade's next write at least
  2 s after the highest timestamp it has written, so that write is stamped above the slider's write
  and above the highest timestamp written before it. In the two `drag-interference` runs with the
  current library, 0 of the probe's 8,122 writes were stamped at or below the highest timestamp
  written before them. The first two runs of that case used an earlier development build that
  stamped 2 s after the stored timestamp. In them, 217 fade writes after a slider write were stamped
  at or below the highest timestamp written before them, so Windows presumably ignored them.
- **When the user releases the slider:** Windows Settings usually turns preview off. The next
  `preview_color_temperature` call then returns `false` and turns preview back on, so the fade
  continues. In one probe run, Windows Settings did not turn preview off on release. A `false`
  return therefore shows only a release that turned preview off, and the return values do not show
  presses, drags, or other releases.
- **After a fade ends:** slider writes apply again. In the first probe run, the 9 slider writes made
  after the fade were stamped above the stored timestamp and changed the screen.
- **During a ramp:** steps are seconds apart, so writes from a slider drag apply between them. The
  next step is stamped 2 s after the slider's latest write, above the slider's writes, and replaces
  the user's temperature. The slider's next write is stamped 2 s after the step if Windows Settings
  has read the step, and otherwise 2 s after its own previous write. In the second case, the
  slider's write has the same timestamp as the step, so Windows ignores it, and the slider's later
  writes are stamped higher and apply. In the probe, the `slider-stamps` case wrote the settings
  back unchanged once per second during one drag. After 7 of its 10 writes, the next slider write
  had the same timestamp as that write, and after the other 3, it was stamped 2 s later. The probe
  did not check what the screen showed during that drag, so the effect on the screen follows from
  the timestamps.

The library does not decide whether a change continues after the user moves the slider. To detect a
slider move before a ramp step, compare the stored temperature from `read_settings` with the last
temperature the application wrote.

## Leftover preview at startup

An application that exits during a held preview leaves preview on, and Windows keeps showing the
stored temperature even while Night Light is off. At startup, call `apply_color_temperature` with
the temperature the application wants on screen, which ends any preview. To keep the stored
temperature instead, read the settings and call `end_preview` if `Settings::is_previewing` returns
`true`. The settings do not show whether a preview was left over or belongs to a drag the user is
making at that moment; ending it cuts off such a drag, and the user can press the slider again.

## When Windows ignores the application's writes

If the application's writes succeed but the screen stops changing, `explorer.exe` is ignoring them,
for example because it has applied a timestamp higher than any the application now writes. In the
probe, two user actions made `explorer.exe` apply writes again:

- **Restart `explorer.exe`.** The new `explorer.exe` applies the stored temperature at startup,
  whatever its timestamp. It then ignores writes without preview until a preview stays on for about
  9 ms or longer, so make the next change with `apply_color_temperature` while Night Light is on.
  The probe ended `explorer.exe` with `taskkill /f /im explorer.exe` and started a new one with
  `explorer.exe`; Task Manager's Restart command for Windows Explorer was not tested. The taskbar
  disappears for a few seconds, and open File Explorer windows close.
- **Sign out and back in.** In [probe finding
  12](../probes/timestamp-rewind/README.md#12-in-one-run-the-display-ignored-settings-writes-until-a-sign-out-for-an-unknown-reason),
  this ended a state in which `explorer.exe` ignored settings writes for an unknown reason. Unlike
  an `explorer.exe` restart, it does not cause a restart block.

The application can suggest these steps to the user. It should not restart `explorer.exe` itself,
because the restart interrupts the user's work.
