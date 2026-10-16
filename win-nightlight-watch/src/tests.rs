use super::*;
use hex_literal::hex;
use std::error::Error as _;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use test_case::test_case;
use windows_sys::Win32::System::Threading::WaitForMultipleObjects;

const WAIT_LIMIT: Duration = Duration::from_secs(2);
const QUIET_WAIT: Duration = Duration::from_millis(20);

const STATE_ACTIVE: &[u8] = &hex!(
    "43420100 0A020100 2A 068995FCBE06 2A 2B0E15"
    "43420100 1000 D00A02 C614A9F6E2D3EFEAE6ED01 00 00 00 00"
);
const SETTINGS_2790: &[u8] = &hex!(
    "43420100 0A020100 2A 06ECA0F4BE06 2A 2B0E26"
    "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF28CC2B CA320E132E1700 CA3C0E072E0C00 00 00 00 00"
);
const SETTINGS_3400: &[u8] = &hex!(
    "43420100 0A020100 2A 06ECA0F4BE06 2A 2B0E26"
    "43420100 0201 C20A00 CA140E012E0F00 CA1E00 CF289035 CA320E132E1700 CA3C0E072E0C00 00 00 00 00"
);

fn watch(path: &str) -> Result<Watcher> {
    Watcher::open(path, || {
        State::from_bytes(STATE_ACTIVE).map(Snapshot::State)
    })
}

struct ScratchKey {
    path: String,
}

impl ScratchKey {
    fn new(name: &str) -> Self {
        let path = format!(
            r"Software\win-nightlight-test\{name}-{}",
            std::process::id()
        );
        drop(
            CURRENT_USER
                .create(&path)
                .expect("create scratch registry key"),
        );
        Self { path }
    }

    fn write_bytes(&self, value: &[u8]) {
        CURRENT_USER
            .options()
            .write()
            .open(&self.path)
            .expect("open scratch key for writing")
            .set_bytes("Data", windows_registry::Type::Bytes, value)
            .expect("write scratch binary value");
    }

    fn write(&self, value: u32) {
        CURRENT_USER
            .options()
            .write()
            .open(&self.path)
            .expect("open scratch key for writing")
            .set_u32("Data", value)
            .expect("write scratch registry value");
    }
}

impl Drop for ScratchKey {
    fn drop(&mut self) {
        if let Err(error) = CURRENT_USER.remove_tree(&self.path) {
            let code = WIN32_ERROR::from_error(&error).map(|code| code.0);
            if !matches!(code, Some(2 | 3)) {
                eprintln!("removing scratch key {}: {error}", self.path);
                assert!(std::thread::panicking(), "scratch registry cleanup failed");
            }
        }
    }
}

#[test]
fn watcher_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Watcher>();
}

#[test_case(Duration::ZERO, 0)]
#[test_case(Duration::from_nanos(1), 1)]
#[test_case(Duration::from_nanos(999_999), 1)]
#[test_case(Duration::from_millis(1), 1)]
#[test_case(Duration::from_nanos(1_000_001), 2)]
#[test_case(Duration::from_millis(4_294_967_294), 4_294_967_294)]
#[test_case(Duration::new(4_294_967, 293_000_001), 4_294_967_294)]
fn timeout_conversion_rounds_up(duration: Duration, expected: u32) {
    assert_eq!(timeout_millis(duration).expect("finite timeout"), expected);
}

#[test_case(Duration::new(4_294_967, 294_000_001))]
#[test_case(Duration::from_millis(4_294_967_295))]
#[test_case(Duration::MAX)]
fn timeout_conversion_rejects_oversized_values(duration: Duration) {
    let error = timeout_millis(duration).expect_err("oversized timeout must fail");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "timeout exceeds 4294967294 milliseconds");
}

#[test]
fn unicode_key_receives_notifications() {
    let scratch = ScratchKey::new("unicode-温度-🌙");
    let watcher = watch(&scratch.path).expect("watch Unicode key");
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("wait for Unicode key")
            .is_some()
    );
}

#[test]
fn timeout_keeps_registration_and_zero_wait_consumes_signal() {
    let scratch = ScratchKey::new("timeout");
    let watcher = watch(&scratch.path).expect("watch scratch key");
    assert!(
        watcher
            .wait_timeout(Duration::ZERO)
            .expect("empty poll")
            .is_none()
    );
    assert!(
        watcher
            .wait_timeout(QUIET_WAIT)
            .expect("bounded timeout")
            .is_none()
    );
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(Duration::ZERO)
            .expect("poll pending signal")
            .is_some()
    );
    assert!(
        watcher
            .wait_timeout(Duration::ZERO)
            .expect("signal consumed")
            .is_none()
    );
}

#[test]
fn invalid_timeout_does_not_consume_notification() {
    let scratch = ScratchKey::new("invalid-timeout");
    let watcher = watch(&scratch.path).expect("watch scratch key");
    scratch.write(1);
    let error = watcher
        .wait_timeout(Duration::MAX)
        .expect_err("reject timeout");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("notification retained")
            .is_some()
    );
}

#[test]
fn another_thread_can_write_the_watched_key() {
    let scratch = ScratchKey::new("other-writer");
    let watcher = watch(&scratch.path).expect("watch scratch key");
    std::thread::scope(|scope| {
        scope
            .spawn(|| scratch.write(1))
            .join()
            .expect("writer thread completed");
    });
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("cross-thread notification")
            .is_some()
    );
}

#[test]
fn registration_survives_the_registering_thread() {
    let scratch = ScratchKey::new("creator-exit");
    let watcher = std::thread::scope(|scope| {
        scope
            .spawn(|| watch(&scratch.path).expect("watch on worker"))
            .join()
            .expect("creator thread completed")
    });
    assert!(
        watcher
            .wait_timeout(QUIET_WAIT)
            .expect("exit must not signal")
            .is_none()
    );
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("notification after exit")
            .is_some()
    );
}

#[test]
fn deleted_key_requires_a_new_watcher() {
    let scratch = ScratchKey::new("deleted");
    let watcher = watch(&scratch.path).expect("watch scratch key");
    CURRENT_USER
        .remove_tree(&scratch.path)
        .expect("delete scratch registry key");
    let error = watcher
        .wait_timeout(WAIT_LIMIT)
        .expect_err("deleted key cannot be rearmed");
    assert_eq!(error.kind(), ErrorKind::KeyDeleted);
    let source = error
        .source()
        .expect("native deletion error")
        .downcast_ref::<windows_result::Error>()
        .expect("Windows error source");
    assert_eq!(WIN32_ERROR::from_error(source), Some(WIN32_ERROR(1018)));
    drop(watcher);

    let error = watch(&scratch.path).expect_err("missing key cannot open");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    drop(
        CURRENT_USER
            .create(&scratch.path)
            .expect("recreate scratch key"),
    );
    let replacement = watch(&scratch.path).expect("watch replacement key");
    scratch.write(1);
    assert!(
        replacement
            .wait_timeout(WAIT_LIMIT)
            .expect("replacement notification")
            .is_some()
    );
}

#[test]
#[expect(
    unsafe_code,
    reason = "exercise external waits on owned watcher events"
)]
fn external_waits_identify_each_key() {
    for borrowed in [false, true] {
        let first = ScratchKey::new(&format!("external-first-{borrowed}"));
        let second = ScratchKey::new(&format!("external-second-{borrowed}"));
        let watchers = [
            watch(&first.path).expect("watch first key"),
            watch(&second.path).expect("watch second key"),
        ];
        let handles = watchers.each_ref().map(|watcher| {
            if borrowed {
                watcher.as_handle().as_raw_handle()
            } else {
                watcher.as_raw_handle()
            }
        });

        for (index, scratch) in [first, second].iter().enumerate() {
            for value in 1..=2 {
                scratch.write(value);
                // SAFETY: Both watcher-owned events and the handle array remain
                // valid for this bounded wait; the array length matches the
                // count.
                let result = unsafe {
                    WaitForMultipleObjects(
                        u32::try_from(handles.len()).expect("handle count fits u32"),
                        handles.as_ptr(),
                        0,
                        2_000,
                    )
                };
                assert_eq!(
                    result,
                    WAIT_OBJECT_0 + u32::try_from(index).expect("watcher index fits u32")
                );
                assert!(
                    watchers[index]
                        .read_pending()
                        .expect("read signaled key")
                        .is_some()
                );
                assert!(
                    watchers[index]
                        .read_pending()
                        .expect("signal consumed once")
                        .is_none()
                );
            }
        }
    }
}

#[test]
#[expect(
    unsafe_code,
    reason = "observe cancellation through a duplicated event"
)]
fn dropping_watcher_cancels_pending_registration() {
    let scratch = ScratchKey::new("drop");
    let watcher = watch(&scratch.path).expect("watch scratch key");
    let event = watcher
        .as_handle()
        .try_clone_to_owned()
        .expect("duplicate notification event");
    drop(watcher);
    // SAFETY: The duplicate owns the same event and remains open for this wait.
    let result = unsafe { WaitForSingleObject(event.as_raw_handle(), 2_000) };
    assert_eq!(
        result, WAIT_OBJECT_0,
        "closing the watched key signals the event"
    );
}

#[test]
fn successful_wait_automatically_registers_the_next_notification() {
    let scratch = ScratchKey::new("automatic-rearm-regression");
    let watcher = watch(&scratch.path).expect("watch scratch key");
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("first notification")
            .is_some()
    );
    scratch.write(2);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("second notification")
            .is_some()
    );
}

#[test]
#[expect(unsafe_code, reason = "check readiness without consuming it")]
fn external_wait_leaves_the_notification_available_to_the_watcher() {
    let scratch = ScratchKey::new("external-ready-regression");
    let watcher = watch(&scratch.path).expect("watch scratch key");
    scratch.write(1);
    // SAFETY: The watcher owns the event throughout this bounded wait.
    let result = unsafe { WaitForSingleObject(watcher.as_raw_handle(), 2_000) };
    assert_eq!(result, WAIT_OBJECT_0);
    assert!(
        watcher
            .wait_timeout(Duration::ZERO)
            .expect("pending notification")
            .is_some()
    );
}

#[test]
fn snapshot_reads_the_latest_settings_after_coalesced_writes() {
    let scratch = ScratchKey::new("latest-settings");
    let path = scratch.path.clone();
    let watcher = Watcher::open(&scratch.path, move || {
        let bytes = CURRENT_USER
            .open(&path)
            .expect("open snapshot key")
            .get_bytes("Data")
            .expect("read snapshot data");
        Settings::from_bytes(&bytes).map(Snapshot::Settings)
    })
    .expect("watch settings fixture");
    scratch.write_bytes(SETTINGS_2790);
    scratch.write_bytes(SETTINGS_3400);
    let Snapshot::Settings(settings) = watcher
        .wait_timeout(WAIT_LIMIT)
        .expect("read changed settings")
        .expect("pending snapshot")
    else {
        panic!("expected settings snapshot");
    };
    assert_eq!(settings.color_temperature(), Some(3400));
    scratch.write_bytes(SETTINGS_2790);
    let Snapshot::Settings(settings) = watcher
        .wait_timeout(WAIT_LIMIT)
        .expect("read subsequent settings")
        .expect("subsequent snapshot")
    else {
        panic!("expected settings snapshot");
    };
    assert_eq!(settings.color_temperature(), Some(2790));
}

#[test]
fn rearming_precedes_the_snapshot_read() {
    let scratch = ScratchKey::new("read-order");
    let path = scratch.path.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = reads.clone();
    let watcher = Watcher::open(&scratch.path, move || {
        if observed.fetch_add(1, Ordering::SeqCst) == 0 {
            CURRENT_USER
                .options()
                .write()
                .open(&path)
                .expect("open during read")
                .set_u32("Data", 2)
                .expect("write during read");
        }
        State::from_bytes(STATE_ACTIVE).map(Snapshot::State)
    })
    .expect("watch scratch key");
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("first read")
            .is_some()
    );
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("change during read")
            .is_some()
    );
    assert_eq!(reads.load(Ordering::SeqCst), 2);
    assert!(watcher.read_pending().expect("no further signal").is_none());
}

#[test]
fn initial_and_explicit_reads_preserve_pending_notifications() {
    let scratch = ScratchKey::new("initial-read");
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = reads.clone();
    let watcher = Watcher::open(&scratch.path, move || {
        observed.fetch_add(1, Ordering::SeqCst);
        State::from_bytes(STATE_ACTIVE).map(Snapshot::State)
    })
    .expect("register before initial read");
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    let Snapshot::State(state) = watcher.read_current().expect("initial snapshot") else {
        panic!("expected state snapshot");
    };
    assert!(state.is_active());
    assert!(
        watcher
            .read_pending()
            .expect("initial read does not signal")
            .is_none()
    );
    scratch.write(1);
    let _snapshot = watcher.read_current().expect("read without consuming");
    assert!(
        watcher
            .read_pending()
            .expect("notification still pending")
            .is_some()
    );
    assert_eq!(reads.load(Ordering::SeqCst), 3);
}

#[test]
fn discarding_and_timeouts_do_not_read_snapshots() {
    let scratch = ScratchKey::new("discard");
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = reads.clone();
    let watcher = Watcher::open(&scratch.path, move || {
        observed.fetch_add(1, Ordering::SeqCst);
        State::from_bytes(b"invalid").map(Snapshot::State)
    })
    .expect("watch scratch key");
    assert!(!watcher.discard_pending().expect("no pending notification"));
    assert!(
        watcher
            .wait_timeout(QUIET_WAIT)
            .expect("timeout does not read")
            .is_none()
    );
    for value in 1..=2 {
        scratch.write(value);
        assert!(watcher.discard_pending().expect("discard pending change"));
        assert!(!watcher.discard_pending().expect("already consumed"));
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    scratch.write(3);
    let error = watcher.read_pending().expect_err("reader now invoked");
    assert_eq!(error.kind(), ErrorKind::Read);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert!(
        watcher
            .read_pending()
            .expect("no automatic read retry")
            .is_none()
    );
}

#[test]
fn failed_snapshot_read_preserves_registration_and_allows_an_explicit_retry() {
    let scratch = ScratchKey::new("read-error");
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = reads.clone();
    let watcher = Watcher::open(&scratch.path, move || {
        let bytes = if observed.fetch_add(1, Ordering::SeqCst) == 0 {
            b"invalid".as_slice()
        } else {
            STATE_ACTIVE
        };
        State::from_bytes(bytes).map(Snapshot::State)
    })
    .expect("watch scratch key");
    scratch.write(1);
    let error = watcher
        .wait_timeout(WAIT_LIMIT)
        .expect_err("first read fails");
    assert_eq!(error.kind(), ErrorKind::Read);
    assert_eq!(error.to_string(), "reading the Night Light snapshot");
    let source = error
        .source()
        .expect("snapshot error source")
        .downcast_ref::<win_nightlight_lib::Error>()
        .expect("typed library error");
    assert_eq!(source.kind(), win_nightlight_lib::ErrorKind::Decode);
    let Snapshot::State(state) = watcher.read_current().expect("retry without another write")
    else {
        panic!("expected state snapshot");
    };
    assert!(state.is_active());
    scratch.write(2);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("still registered")
            .is_some()
    );
    assert_eq!(reads.load(Ordering::SeqCst), 3);
}

#[test_case(false; "read pending")]
#[test_case(true; "discard pending")]
fn registration_failure_does_not_read_a_snapshot(discard: bool) {
    let scratch = ScratchKey::new(&format!("registration-error-{discard}"));
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = reads.clone();
    let watcher = Watcher::open(&scratch.path, move || {
        observed.fetch_add(1, Ordering::SeqCst);
        State::from_bytes(STATE_ACTIVE).map(Snapshot::State)
    })
    .expect("watch scratch key");
    CURRENT_USER
        .remove_tree(&scratch.path)
        .expect("delete watched key");
    let error = if discard {
        watcher.discard_pending().map(|_| ())
    } else {
        watcher.read_pending().map(|_| ())
    }
    .expect_err("registration fails before reading");
    assert_eq!(error.kind(), ErrorKind::KeyDeleted);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}
