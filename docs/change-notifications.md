# Change notifications

`win-nightlight-watch` observes value changes in one existing registry key under
`HKEY_CURRENT_USER`. Use it with `win-nightlight-lib` to refresh Night Light state or settings
when Windows or another application changes them. Both crates support Windows only.

The watcher opens the key with notification access and creates one auto-reset event. It does
not read or write registry values, create missing keys, or start a background thread.

## Watch Night Light

Add both dependencies:

```shell
cargo add win-nightlight-lib win-nightlight-watch
```

`win-nightlight-lib` exports `SETTINGS_KEY_PATH` and `STATE_KEY_PATH`, relative to
`HKEY_CURRENT_USER`. Each key stores its Night Light payload in the `Data` value. Create a
`Watcher` for each key you need before the initial `Nightlight` read.

Each watcher has exactly one notification consumer. Use this sequence:

1. Wait with `wait_timeout(Duration)` or an external Windows wait on the event handle.
2. If the wait times out, wait again without rearming.
3. If the wait consumes a notification, call `rearm()` exactly once. On any rearm error,
   drop the watcher and stop using that instance.
4. Read the current value through `Nightlight`, then repeat the wait.

See the [crate documentation](https://docs.rs/win-nightlight-watch) for a compiled usage
example that follows this order.

Notifications can combine several writes. They indicate that the key needs rereading;
they do not identify a value or count writes. Changes between consuming a signal and rearming
need not signal again, but the following read retrieves current data. Changes after rearming
can signal the next notification.

Calling `rearm()` while the previous registration is pending allocates additional native wait
operations. External waits consume the event without updating Rust-side state, so the caller
must enforce the sequence above.

## Integrate with a Windows event loop

`Watcher` implements `AsHandle` and `AsRawHandle`. Wait on both Night Light watchers with a
wait-any operation to identify which key needs rereading. Keep the watchers alive during the
wait, and rearm only the watcher whose notification was consumed.

A thread that processes window messages must use a message-aware wait, such as
`MsgWaitForMultipleObjectsEx`, instead of blocking in `wait_timeout`. Other threads can use
`wait_timeout` or `WaitForMultipleObjects`. The watcher is `Send` but not `Sync`; registration
survives the thread that created it.

## Timeouts

`wait_timeout` performs one native wait. Zero polls immediately; positive fractional
milliseconds round up. Durations above 4,294,967,294 milliseconds return `InvalidInput` before
waiting. Finite durations never become infinite waits. Time spent asleep does not count
toward the native timeout on Windows 11.

## Errors and recovery

Inspect `Error::kind()` without parsing messages:

| Kind | Meaning |
| :--- | :------ |
| `NotFound` | The key does not exist. |
| `KeyDeleted` | Windows reports that the watched key was deleted. |
| `InvalidInput` | The path is empty or has a NUL character, or the timeout is too large. |
| `Windows` | Another registry or event operation failed. |

The error source chain preserves underlying Windows errors and their native codes.

After any rearm error, drop the watcher. The application can attempt to create a replacement,
but reopening fails while the key is absent. An existing watcher does not follow a replacement
key at the same path. The crate does not recreate keys or retry automatically.
