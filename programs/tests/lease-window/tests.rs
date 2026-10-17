//! The adapter's window dispatch, from a parent that was **not** pre-narrowed.
//!
//! Witnessed here:
//! - a `Some((offset, count))` assignment goes through `env::pie::accord_window` with
//!   exactly those coordinates, and the derived token opens at exactly that range;
//! - a `None` assignment goes through plain `env::pie::accord` and inherits the parent.
//!
//! Not witnessed, and not claimed: the `inspect`/`revoke` halves of the adapter (they
//! reach the real `resource`, which is an ecall on the host) and the `PrivateRoot::derive`
//! gate around them (`derive_with` with a fake effects model is covered in the supply
//! fixture; here it is the native adapter itself that is driven, directly).

use crate::lease::{Accord, NativePieEffects, PieEffects};
use crate::{Call, Mark, Permission, PieToken, TaskId};

/// A legitimate parent that was **not** pre-narrowed: it owns the whole eight pages.
const PARENT: (usize, usize) = (0, 8);
/// The assignment narrows to this range; nothing else can produce it.
const ASSIGNED: (usize, usize) = (1, 3);

fn request(parent: PieToken, window: Option<(usize, usize)>) -> Accord {
    Accord {
        token: parent,
        target: TaskId::new(8),
        permission: Permission::FETCH,
        mark: Mark::of("lease-root"),
        window,
    }
}

#[test]
fn the_native_adapter_accords_the_assigned_window_and_not_the_whole_parent() {
    let parent = PieToken::mint(1);
    crate::seed(parent, PARENT);
    let child = NativePieEffects
        .accord(request(parent, Some(ASSIGNED)))
        .expect("the stand-in accepts the call");
    // The windowed path was taken, with exactly the assigned coordinates.
    assert_eq!(
        crate::calls(),
        vec![Call::Window(parent, ASSIGNED.0, ASSIGNED.1)],
        "the assignment's window must reach `accord_window`"
    );
    // What `Open` would hand back from the derived token is the assigned range...
    assert_eq!(crate::window(child), ASSIGNED);
    // ...and not the parent's, which the parent still holds unchanged. This is the
    // assertion an adapter that accords both branches (inheriting the whole parent)
    // cannot satisfy.
    assert_eq!(crate::window(parent), PARENT);
    assert_ne!(crate::window(child), crate::window(parent));
}

#[test]
fn the_native_adapter_without_a_window_inherits_the_parent() {
    let parent = PieToken::mint(2);
    crate::seed(parent, PARENT);
    let child = NativePieEffects
        .accord(request(parent, None))
        .expect("the stand-in accepts the call");
    assert_eq!(
        crate::calls(),
        vec![Call::Accord(parent)],
        "no window asked for takes the inheriting path"
    );
    assert_eq!(
        crate::window(child),
        PARENT,
        "no window asked for = the parent's range"
    );
}
