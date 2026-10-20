use crate::{PAGE_SIZE, PieToken, pages::Pages, state};

fn take() -> Pages {
    let mut pages = Pages::default();
    pages.take(PieToken::mint(7), 2).unwrap();
    assert_eq!(state::counts(), (1, 1, 1));
    pages
}

#[test]
fn closes_in_order_and_repeated_close_is_empty() {
    let mut pages = take();
    assert_eq!(pages.count(), 2);
    assert_eq!(pages.source(), PieToken::mint(11));
    assert_eq!(pages.physical(0), PAGE_SIZE);
    assert_eq!(pages.physical(1), PAGE_SIZE * 2);
    assert_eq!(pages.get8(0, 0), 0);
    assert_eq!(pages.get32(1, PAGE_SIZE - 4), 0);
    pages.put8(0, 0, 7);
    pages.put16(0, 2, 123);
    pages.put32(0, 4, 456);
    pages.put64(0, 8, 789);
    assert_eq!(pages.get8(0, 0), 7);
    assert_eq!(pages.get16(0, 2), 123);
    assert_eq!(pages.get32(0, 4), 456);
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
    assert_eq!(pages.count(), 0);
    let events = state::events();
    assert!(events.ends_with(&["close view", "release reference", "release pole"]));
    pages.close().unwrap();
    assert_eq!(state::events(), events);
}

#[test]
fn view_failure_keeps_all_ownership_until_retry() {
    let mut pages = take();
    state::fail(Some("close view"));
    for _ in 0..2 {
        assert_eq!(pages.close(), Err("iommu: view close"));
        assert_eq!(state::counts(), (1, 1, 1));
    }
    state::fail(None);
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn reference_failure_does_not_close_view_twice() {
    let mut pages = take();
    state::fail(Some("release reference"));
    for _ in 0..2 {
        assert_eq!(pages.close(), Err("iommu: page release"));
        assert_eq!(state::counts(), (1, 1, 0));
    }
    state::fail(None);
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
    assert_eq!(
        state::events()
            .iter()
            .filter(|event| **event == "close view")
            .count(),
        1
    );
}

#[test]
fn pole_failure_keeps_only_unreleased_source() {
    let mut pages = take();
    state::fail(Some("release pole"));
    for _ in 0..2 {
        assert_eq!(pages.close(), Err("iommu: page source"));
        assert_eq!(state::counts(), (1, 0, 0));
    }
    state::fail(None);
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
    let events = state::events();
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == "close view")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == "release reference")
            .count(),
        1
    );
}

fn failed_take(operation: &'static str, counts: (usize, usize, usize)) {
    state::fail(Some(operation));
    let mut pages = Pages::default();
    assert!(pages.take(PieToken::mint(7), 2).is_err());
    assert_eq!(state::counts(), counts);
    state::fail(None);
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn failed_pole_has_nothing_to_release() {
    failed_take("pole", (0, 0, 0));
}

#[test]
fn failed_acquire_keeps_pole_for_cleanup() {
    failed_take("acquire", (1, 0, 0));
}

#[test]
fn failed_attach_keeps_reference_and_pole_for_cleanup() {
    failed_take("attach", (1, 1, 0));
}

#[test]
fn failed_handle_keeps_view_reference_and_pole_for_cleanup() {
    failed_take("handle", (1, 1, 1));
}

#[test]
fn failed_traverse_keeps_view_reference_and_pole_for_cleanup() {
    failed_take("traverse", (1, 1, 1));
}

#[test]
fn failed_construction_and_failed_cleanup_keep_the_same_pole() {
    state::fail(Some("attach"));
    let mut pages = Pages::default();
    assert_eq!(pages.take(PieToken::mint(7), 2), Err("iommu: page view"));
    state::fail(Some("release pole"));
    assert_eq!(pages.close(), Err("iommu: page source"));
    assert_eq!(state::counts(), (1, 0, 0));
    assert_eq!(pages.source(), PieToken::mint(11));
    state::fail(None);
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn invalid_counts_do_not_start_acquisition() {
    let mut pages = Pages::default();
    for count in [0, usize::MAX] {
        assert_eq!(
            pages.take(PieToken::mint(7), count),
            Err("iommu: page count")
        );
    }
    assert!(state::events().is_empty());
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}

#[test]
fn live_or_partially_acquired_resources_cannot_be_overwritten() {
    state::fail(Some("attach"));
    let mut pages = Pages::default();
    assert!(pages.take(PieToken::mint(7), 2).is_err());
    let events = state::events();
    assert_eq!(
        pages.take(PieToken::mint(7), 3),
        Err("iommu: pages already owned")
    );
    assert_eq!(state::events(), events);
    state::fail(None);
    pages.close().unwrap();
    pages.take(PieToken::mint(7), 3).unwrap();
    let events = state::events();
    assert_eq!(
        pages.take(PieToken::mint(7), 4),
        Err("iommu: pages already owned")
    );
    assert_eq!(state::events(), events);
    assert_eq!(pages.count(), 3);
    pages.close().unwrap();
    assert_eq!(state::counts(), (0, 0, 0));
}
