use super::*;

static RUN: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn slay_at_commit_boundary_cannot_be_overwritten_by_waiting() {
    let _run = RUN.lock().unwrap();
    reset();
    INJECT.set(true);
    let result = block::block_with_call::<env::RoomFail>(
        1, Weak::new(), Duration::from_millis(1), None,
    );
    assert_eq!(result, Ok(Blocked::Quit(123)));
    assert_eq!(OPS.with(|ops| ops.borrow().clone()), ["slay", "commit", "check"]);
    assert!(sites(1).lock().is_empty());
}

#[test]
fn live_wait_is_published_under_the_same_commit() {
    let _run = RUN.lock().unwrap();
    reset();
    let result = block::block_with_call::<env::RoomFail>(
        1, Weak::new(), Duration::from_millis(1), Some(()),
    );
    assert_eq!(result, Ok(Blocked::Switch(Some(42))));
    assert_eq!(OPS.with(|ops| ops.borrow().clone()), [
        "commit", "check", "ensure", "hold", "alarm", "swap", "trace", "block", "enqueue",
    ]);
}
