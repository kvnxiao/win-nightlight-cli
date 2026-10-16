use super::*;
use std::error::Error as _;
use test_case::test_case;
use windows_sys::Win32::System::Threading::WaitForMultipleObjects;

const WAIT_LIMIT: Duration = Duration::from_secs(2);
const QUIET_WAIT: Duration = Duration::from_millis(20);

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

#[test_case("")]
#[test_case("\0")]
#[test_case("\0Software")]
fn invalid_paths_are_rejected(path: &str) {
    let error = Watcher::new(path).expect_err("invalid path must fail");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
}

#[test]
fn embedded_nul_does_not_open_the_existing_prefix() {
    let scratch = ScratchKey::new("embedded-nul");
    let path = format!("{}\0missing", scratch.path);
    let error = Watcher::new(&path).expect_err("embedded NUL must fail");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
}

#[test]
fn unicode_key_receives_notifications() {
    let scratch = ScratchKey::new("unicode-温度-🌙");
    let watcher = Watcher::new(&scratch.path).expect("watch Unicode key");
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("wait for Unicode key")
    );
}

#[test]
fn notification_is_consumed_and_requires_rearming() {
    let scratch = ScratchKey::new("rearm");
    let watcher = Watcher::new(&scratch.path).expect("watch scratch key");
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("first notification")
    );
    assert!(
        !watcher
            .wait_timeout(Duration::ZERO)
            .expect("consume signal")
    );

    scratch.write(2);
    assert!(
        !watcher
            .wait_timeout(QUIET_WAIT)
            .expect("one-shot registration")
    );

    watcher.rearm().expect("rearm after consumed notification");
    scratch.write(3);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("rearmed notification")
    );
}

#[test]
fn timeout_keeps_registration_and_zero_wait_consumes_signal() {
    let scratch = ScratchKey::new("timeout");
    let watcher = Watcher::new(&scratch.path).expect("watch scratch key");
    assert!(!watcher.wait_timeout(Duration::ZERO).expect("empty poll"));
    assert!(!watcher.wait_timeout(QUIET_WAIT).expect("bounded timeout"));
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(Duration::ZERO)
            .expect("poll pending signal")
    );
    assert!(
        !watcher
            .wait_timeout(Duration::ZERO)
            .expect("signal consumed")
    );
}

#[test]
fn invalid_timeout_does_not_consume_notification() {
    let scratch = ScratchKey::new("invalid-timeout");
    let watcher = Watcher::new(&scratch.path).expect("watch scratch key");
    scratch.write(1);
    let error = watcher
        .wait_timeout(Duration::MAX)
        .expect_err("reject timeout");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("notification retained")
    );
}

#[test]
fn another_thread_can_write_the_watched_key() {
    let scratch = ScratchKey::new("other-writer");
    let watcher = Watcher::new(&scratch.path).expect("watch scratch key");
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
    );
}

#[test]
fn registration_survives_the_registering_thread() {
    let scratch = ScratchKey::new("creator-exit");
    let watcher = std::thread::scope(|scope| {
        scope
            .spawn(|| Watcher::new(&scratch.path).expect("watch on worker"))
            .join()
            .expect("creator thread completed")
    });
    assert!(
        !watcher
            .wait_timeout(QUIET_WAIT)
            .expect("exit must not signal")
    );
    scratch.write(1);
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("notification after exit")
    );
}

#[test]
fn deleted_key_requires_a_new_watcher() {
    let scratch = ScratchKey::new("deleted");
    let watcher = Watcher::new(&scratch.path).expect("watch scratch key");
    CURRENT_USER
        .remove_tree(&scratch.path)
        .expect("delete scratch registry key");
    assert!(
        watcher
            .wait_timeout(WAIT_LIMIT)
            .expect("deletion notification")
    );
    let error = watcher.rearm().expect_err("deleted key cannot be rearmed");
    assert_eq!(error.kind(), ErrorKind::KeyDeleted);
    let source = error
        .source()
        .expect("native deletion error")
        .downcast_ref::<windows_result::Error>()
        .expect("Windows error source");
    assert_eq!(WIN32_ERROR::from_error(source), Some(WIN32_ERROR(1018)));
    drop(watcher);

    let error = Watcher::new(&scratch.path).expect_err("missing key cannot open");
    assert_eq!(error.kind(), ErrorKind::NotFound);
    drop(
        CURRENT_USER
            .create(&scratch.path)
            .expect("recreate scratch key"),
    );
    let replacement = Watcher::new(&scratch.path).expect("watch replacement key");
    scratch.write(1);
    assert!(
        replacement
            .wait_timeout(WAIT_LIMIT)
            .expect("replacement notification")
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
            Watcher::new(&first.path).expect("watch first key"),
            Watcher::new(&second.path).expect("watch second key"),
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
                watchers[index]
                    .rearm()
                    .expect("rearm externally consumed signal");
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
    let watcher = Watcher::new(&scratch.path).expect("watch scratch key");
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
