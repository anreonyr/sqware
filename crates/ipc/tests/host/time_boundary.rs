use crate::Wait;
use ipc_impl::time::Deadline;
use std::cell::Cell;

#[test]
fn new_reads_clock_once_for_forever_zero_and_finite() {
    for wait in [Wait::Forever, Wait::AtMost(0), Wait::AtMost(7)] {
        let reads = Cell::new(0);
        let _ = Deadline::new_with_clock(wait, || {
            reads.set(reads.get() + 1);
            42
        });
        assert_eq!(reads.get(), 1);
    }
}

#[test]
fn remaining_forever_reads_no_clock() {
    let deadline = Deadline::new_with_clock(Wait::Forever, || 1);
    assert_eq!(
        deadline.remaining_with_clock(|| panic!("forever remaining read the clock")),
        Wait::Forever
    );
}

#[test]
fn remaining_zero_budget_reads_clock_once() {
    let deadline = Deadline::new_with_clock(Wait::AtMost(0), || 0);
    let reads = Cell::new(0);
    assert_eq!(
        deadline.remaining_with_clock(|| {
            reads.set(reads.get() + 1);
            0
        }),
        Wait::AtMost(0)
    );
    assert_eq!(reads.get(), 1);
}

#[test]
fn remaining_preserves_budget_when_clock_goes_backwards() {
    let deadline = Deadline::new_with_clock(Wait::AtMost(5), || 10_000_000);
    assert_eq!(
        deadline.remaining_with_clock(|| 9_000_000),
        Wait::AtMost(5)
    );
}

#[test]
fn remaining_floors_nanoseconds_at_millisecond_boundaries() {
    let deadline = Deadline::new_with_clock(Wait::AtMost(10), || 0);
    assert_eq!(
        deadline.remaining_with_clock(|| 999_999),
        Wait::AtMost(10)
    );
    assert_eq!(
        deadline.remaining_with_clock(|| 1_000_000),
        Wait::AtMost(9)
    );
    assert_eq!(
        deadline.remaining_with_clock(|| 1_000_001),
        Wait::AtMost(9)
    );
}

#[test]
fn remaining_saturates_exhaustion_and_usize_max_budget() {
    let exhausted = Deadline::new_with_clock(Wait::AtMost(3), || 0);
    assert_eq!(
        exhausted.remaining_with_clock(|| 10_000_000),
        Wait::AtMost(0)
    );

    let maximum = Deadline::new_with_clock(Wait::AtMost(usize::MAX), || 0);
    assert_eq!(
        maximum.remaining_with_clock(|| 2_000_000_000),
        Wait::AtMost(usize::MAX - 2_000)
    );
}

#[test]
fn finite_clock_panic_propagates_from_new_and_remaining() {
    let new = std::panic::catch_unwind(|| {
        Deadline::new_with_clock(Wait::AtMost(1), || panic!("new clock failed"))
    });
    assert!(new.is_err());

    let deadline = Deadline::new_with_clock(Wait::AtMost(1), || 0);
    let remaining = std::panic::catch_unwind(|| {
        deadline.remaining_with_clock(|| panic!("remaining clock failed"))
    });
    assert!(remaining.is_err());
}

#[test]
fn public_deadline_keeps_old_traits_signatures_and_inference() {
    fn traits<T: Copy + Clone + core::fmt::Debug + PartialEq + Eq + Send + Sync>() {}
    traits::<Deadline>();

    fn old_new(wait: Wait) -> Deadline {
        Deadline::new(wait)
    }
    fn old_remaining(deadline: Deadline) -> Wait {
        deadline.remaining()
    }
    let _: fn(Wait) -> Deadline = old_new;
    let _: fn(Deadline) -> Wait = old_remaining;
    let _: fn(Wait) -> Deadline = Deadline::new;
    let _: fn(Deadline) -> Wait = Deadline::remaining;

    let inferred = || {
        let deadline = Deadline::new(Wait::AtMost(0));
        let _: Deadline = deadline;
    };
    let _ = inferred as fn();
}
