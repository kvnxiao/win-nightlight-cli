# Change notifications

`win-nightlight-watch` waits for Night Light changes and returns decoded settings or state through
`win-nightlight-lib`. A snapshot describes the value when the read occurs, which can be later than
the change that signaled the watcher. Both crates support Windows only.

The watcher does not write registry values, create missing keys, or start a background thread.
The calling thread performs each wait and read.

## Watch Night Light

Add the watcher dependency:

```shell
cargo add win-nightlight-watch
```

Use `Watcher::settings()` for `Snapshot::Settings(win_nightlight_lib::Settings)` or
`Watcher::state()` for `Snapshot::State(win_nightlight_lib::State)`. Each constructor registers
for changes to the corresponding Night Light registry key without reading its initial value.
If you need to name the decoded types directly, also add `win-nightlight-lib` as a dependency.

Create every watcher you need before calling `read_current()` on each for the initial data.
Registering first allows changes during the initial reads to signal a notification.
`read_current()` does not consume a pending notification or change registration.

For a blocking loop, call `wait_timeout(Duration)`:

1. On a notification, the watcher resets the event and registers for the next change before
   reading the current record. It returns `Some(Snapshot)` after a successful read.
2. On timeout, it returns `None` without reading or changing registration. Wait again when ready.
3. On an error, follow the [recovery contract](#errors-and-recovery) below.

See the [crate documentation](https://docs.rs/win-nightlight-watch) for a compiled usage example.

## Snapshot consistency

The native registry notification reports that the key changed; it does not include the written
value. The watcher reads the current `Data` value using the library's public read operations.

- Multiple writes can produce one notification, so intermediate values may be omitted.
- A write after rearming can appear in the current read and signal the next notification.
  Repeated snapshots can therefore describe the same data; the watcher does not deduplicate them.
- Settings and state are separate records. Reading both does not produce an atomic pair, and
  neither record proves that Windows applied the corresponding display output.

Changes between consuming a signal and rearming need not signal again, but the following read
retrieves current data. Another writer can change the value again before the caller receives it.
Applications that make control decisions must perform any fresh reads their own contracts require.

## Integrate with a Windows event loop

`Watcher` implements `AsHandle` and `AsRawHandle` for its manual-reset event. External waits observe
readiness without consuming the signal. Use a wait-any operation for settings and state, keep both
watchers alive during the wait, and call `read_pending()` on the signaled watcher. That method
consumes the signal, rearms, and reads the current snapshot. It returns `None` without reading or
rearming if no signal is pending. A successful snapshot read uses `Some(Snapshot)`.

Each watcher has exactly one notification consumer. Callers must not reset or signal its event or
register notifications on its behalf. The watcher is `Send` but not `Sync`; notification
registration survives the thread that created it.

A thread that processes window messages must use a message-aware wait, such as
`MsgWaitForMultipleObjectsEx`, instead of blocking in `wait_timeout`. Other threads can use
`wait_timeout` or `WaitForMultipleObjects`. Snapshot reads still run synchronously on the calling
thread, including after an external wait.

## Defer reads and retry later

If application policy temporarily forbids reads, call `discard_pending()` on a signaled watcher.
It consumes the notification and rearms without reading a snapshot, returning `true` when it
consumed a signal or `false` when none was pending. The watcher does not save discarded changes
for later delivery.

Call `read_current()` when the application chooses to retry or resynchronize, even if no new
notification has arrived. A successful read does not consume any pending signal, so the next
notification read may return the same data. The watcher does not schedule retries.

## Timeouts

The duration passed to `wait_timeout` bounds one native wait, not the subsequent snapshot read.
Zero polls immediately; positive fractional milliseconds round up. Durations above 4,294,967,294
milliseconds return `InvalidInput` before waiting. Finite durations never become infinite waits.
Time spent asleep does not count toward the native timeout on Windows 11.

## Errors and recovery

Inspect `Error::kind()` without parsing messages:

| Kind | Meaning and action |
| :--- | :----------------- |
| `Read` | Reading or decoding the snapshot failed. Registration is preserved; retry or defer reads according to application policy. |
| `InvalidInput` | The timeout is too large. Use a valid duration; the notification was not consumed. |
| `NotFound` | The key does not exist. A constructor cannot create a watcher until the key exists. Drop an existing watcher if a notification operation reports this error. |
| `KeyDeleted` | Windows reports that the watched key was deleted. Drop the watcher. |
| `Windows` | Another registry or event operation failed. Drop the watcher if construction had succeeded. |

A `Read` error preserves the underlying `win_nightlight_lib::Error` in its source chain. Reads use
the library's bounded retries for concurrent value changes; the watcher does not add a retry loop.
For notification-driven reads, rearming succeeds before the read starts, so a read failure does
not stop future notifications. Explicit `read_current()` calls do not change registration.

A native wait, event-reset, or registration failure stops the operation before reading a snapshot.
The error source chain preserves underlying Windows errors and their native codes when available.
After such a failure, drop the watcher. The application can attempt to create a replacement, but
reopening fails while the key is absent. An existing watcher does not follow a replacement key at
the same path. The crate does not recreate keys or reopen watchers automatically.
