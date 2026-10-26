use std::sync::Mutex;

use env::{
    AcquireSource, FlatReference, MemoryFail, PieToken, Placement, TeamId, ViewControl,
    ViewSelection,
};

use super::PAGE_SIZE;
use super::reference::Reference;
use super::view::View;
use crate::fixture_kernel::FixtureKernel;
use crate::fixture_state;

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn source() -> AcquireSource {
    AcquireSource::Source {
        token: PieToken::mint(1),
        offset: 0,
        count: 1,
    }
}

fn acquire() -> Reference<FixtureKernel> {
    Reference::acquire(source(), PieToken::NONE).expect("fixture acquire")
}

fn placement() -> Placement {
    Placement::lowest(PAGE_SIZE).expect("page placement")
}

fn owning_view() -> (View<FixtureKernel>, FlatReference) {
    let reference = acquire();
    let handle = reference.handle().expect("fixture handle");
    let mut view = View::<FixtureKernel>::new(
        TeamId::new(3),
        fixture_state::with(|state| state.view),
        fixture_state::with(|state| state.address),
        PAGE_SIZE,
    );
    view.pending = Some(reference);
    (view, handle)
}

#[test]
fn acquire_failure_does_not_create_an_owner() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    fixture_state::with(|state| state.fail_acquire = true);
    let error = Reference::<FixtureKernel>::acquire(source(), PieToken::NONE)
        .err()
        .expect("acquire failure");
    assert!(matches!(error.source, MemoryFail::Denied));
    assert_eq!(fixture_state::with(|state| state.acquire_calls), 1);
    assert!(fixture_state::with(|state| state.live.is_empty()));
}

#[test]
fn release_failure_keeps_the_same_handle_until_a_successful_retry() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    let mut reference = acquire();
    let handle = reference.handle().expect("fixture handle");
    fixture_state::with(|state| state.fail_release = true);
    let error = reference.release().expect_err("release failure");
    assert!(matches!(error.source, MemoryFail::Denied));
    assert_eq!(reference.handle().expect("retained handle"), handle);
    assert_eq!(fixture_state::with(|state| state.release_calls), 1);
    assert_eq!(
        fixture_state::with(|state| state.live.clone()),
        vec![handle.get()]
    );

    fixture_state::with(|state| state.fail_release = false);
    reference.release().expect("release retry");
    assert!(reference.handle().is_err(), "success clears the handle");
    assert_eq!(fixture_state::with(|state| state.release_calls), 2);
    assert!(fixture_state::with(|state| state.live.is_empty()));

    let error = reference
        .release()
        .expect_err("released reference is denied");
    assert!(matches!(error.source, MemoryFail::Denied));
    assert_eq!(
        fixture_state::with(|state| state.release_calls),
        2,
        "a released reference does not call the kernel again"
    );
}

#[test]
fn dropping_a_reference_does_not_release_it() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    let reference = acquire();
    let handle = reference.handle().expect("fixture handle");
    drop(reference);
    assert_eq!(fixture_state::with(|state| state.release_calls), 0);
    assert_eq!(
        fixture_state::with(|state| state.live.clone()),
        vec![handle.get()]
    );
}

#[test]
fn invalid_attach_coordinates_never_call_the_kernel() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    let reference = acquire();
    let handle = reference.handle().expect("fixture handle");
    for (offset, count) in [(0, 0), (0, usize::MAX), (usize::MAX, 1)] {
        let error = reference
            .attach(
                TeamId::new(0),
                offset,
                count,
                placement(),
                6,
                ViewControl::Caller,
            )
            .err()
            .expect("invalid attach");
        assert!(matches!(error.source, MemoryFail::NotAligned));
    }
    assert_eq!(fixture_state::with(|state| state.attach_calls), 0);
    assert_eq!(reference.handle().expect("retained handle"), handle);
}

#[test]
fn attach_failure_keeps_the_reference_and_returns_the_original_error() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    let reference = acquire();
    let handle = reference.handle().expect("fixture handle");
    fixture_state::with(|state| state.fail_attach = true);
    let error = reference
        .attach(TeamId::new(3), 1, 2, placement(), 6, ViewControl::Caller)
        .err()
        .expect("attach failure");
    assert!(matches!(error.source, MemoryFail::NoRegion));
    assert_eq!(fixture_state::with(|state| state.attach_calls), 1);
    assert_eq!(reference.handle().expect("retained handle"), handle);
    assert_eq!(
        fixture_state::with(|state| state.live.clone()),
        vec![handle.get()]
    );
}

#[test]
fn attach_success_and_set_access_keep_the_kernel_error_payload() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    let reference = acquire();
    let handle = reference.handle().expect("fixture handle");
    let mut view = reference
        .attach(TeamId::new(3), 1, 2, placement(), 6, ViewControl::Caller)
        .expect("attach");
    let view_id = fixture_state::with(|state| state.view);
    assert_eq!(view.id().expect("view id"), view_id);
    assert_eq!(view.address(), fixture_state::with(|state| state.address));
    assert_eq!(view.size(), 2 * PAGE_SIZE);
    assert_eq!(
        fixture_state::with(|state| state.last_range),
        Some((handle.get(), 1, 2))
    );

    fixture_state::with(|state| state.fail_set_access = true);
    let error = view
        .set_access(ViewSelection::Whole, 7)
        .expect_err("set_access failure");
    assert!(matches!(error.source, MemoryFail::NoRegion));
    assert_eq!(fixture_state::with(|state| state.set_access_calls), 1);
    assert_eq!(
        fixture_state::with(|state| state.last_access),
        Some((TeamId::new(3).get(), view_id.get(), ViewSelection::Whole, 7))
    );

    fixture_state::with(|state| state.fail_set_access = false);
    view.set_access(ViewSelection::Whole, 7)
        .expect("set_access retry");
    assert_eq!(fixture_state::with(|state| state.set_access_calls), 2);
    view.close().expect("view close");
    assert_eq!(fixture_state::with(|state| state.detach_calls), 1);
    assert_eq!(fixture_state::with(|state| state.release_calls), 0);
}

#[test]
fn detach_failure_keeps_the_id_and_pending_without_releasing() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    let (mut view, handle) = owning_view();
    let view_id = view.id().expect("view id");
    fixture_state::with(|state| state.fail_detach = true);
    let error = view.close().expect_err("detach failure");
    assert!(matches!(error.source, MemoryFail::NoRegion));
    assert_eq!(view.id().expect("retained id"), view_id);
    assert!(view.pending.is_some(), "pending is retained");
    assert_eq!(fixture_state::with(|state| state.detach_calls), 1);
    assert_eq!(fixture_state::with(|state| state.release_calls), 0);
    assert_eq!(
        fixture_state::with(|state| state.live.clone()),
        vec![handle.get()]
    );
}

#[test]
fn pending_release_failure_after_detach_retries_only_the_release() {
    let _test = TEST_LOCK.lock().unwrap();
    fixture_state::reset();
    let (mut view, handle) = owning_view();
    fixture_state::with(|state| {
        state.fail_detach = true;
    });
    view.close().expect_err("detach failure");

    fixture_state::with(|state| {
        state.fail_detach = false;
        state.fail_release = true;
    });
    let error = view.close().expect_err("pending release failure");
    assert!(matches!(error.source, MemoryFail::Denied));
    assert!(view.id().is_err(), "detach success clears the id");
    assert!(view.pending.is_some(), "pending is retained");
    assert_eq!(fixture_state::with(|state| state.detach_calls), 2);
    assert_eq!(fixture_state::with(|state| state.release_calls), 1);
    assert_eq!(
        fixture_state::with(|state| state.live.clone()),
        vec![handle.get()]
    );

    fixture_state::with(|state| state.fail_release = false);
    view.close().expect("release retry");
    assert!(view.pending.is_none());
    assert!(fixture_state::with(|state| state.live.is_empty()));
    assert_eq!(
        fixture_state::with(|state| state.detach_calls),
        2,
        "the retry does not detach twice"
    );
    assert_eq!(fixture_state::with(|state| state.release_calls), 2);

    view.close().expect("closed view is stable");
    assert_eq!(fixture_state::with(|state| state.detach_calls), 2);
    assert_eq!(fixture_state::with(|state| state.release_calls), 2);
}
