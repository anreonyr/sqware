//! The adapter's local-derive and cross-task-accord dispatch, from a parent that was
//! **not** pre-narrowed.
//!
//! Witnessed here:
//! - a `Pages { offset, count }` assignment goes through `env::pie::derive` with exactly
//!   those coordinates, and the derived token opens at exactly that range;
//! - an `Inherit` assignment goes through `env::pie::derive` and keeps the parent's range;
//! - a cross-task delivery goes through plain `env::pie::accord` and inherits the parent.
//!
//! Not witnessed, and not claimed: the `inspect`/`revoke` halves of the adapter (they
//! reach the real `resource`, which is an ecall on the host) and the `PrivateRoot::derive`
//! gate around them (`derive_with` with a fake effects model is covered in the supply
//! fixture; here it is the native adapter itself that is driven, directly).

use crate::derivation::Derivation;
use crate::lease::{Accord, NativePieEffects, PieEffects};
use crate::{Call, GrantRange, Mark, Permission, PieToken, TaskId};

/// A legitimate parent that was **not** pre-narrowed: it owns the whole eight pages.
const PARENT: (usize, usize) = (0, 8);
/// The assignment narrows to this range; nothing else can produce it.
const ASSIGNED: (usize, usize) = (1, 3);

fn derivation(parent: PieToken, range: GrantRange) -> Derivation {
    Derivation {
        source: parent,
        permission: Permission::FETCH,
        mark: Mark::of("lease-root"),
        range,
    }
}

#[test]
fn the_native_adapter_derives_the_assigned_window_and_not_the_whole_parent() {
    let parent = PieToken::mint(1);
    crate::seed(parent, PARENT);
    let child = NativePieEffects
        .derive(derivation(
            parent,
            GrantRange::Pages {
                offset: ASSIGNED.0,
                count: ASSIGNED.1,
            },
        ))
        .expect("the stand-in accepts the call");
    assert_eq!(
        crate::calls(),
        vec![Call::Derive(
            parent,
            GrantRange::Pages {
                offset: ASSIGNED.0,
                count: ASSIGNED.1
            }
        )],
        "the assignment's range must reach `derive`"
    );
    // What `Open` would hand back from the derived token is the assigned range...
    assert_eq!(crate::window(child), ASSIGNED);
    // ...and not the parent's, which the parent still holds unchanged.
    assert_eq!(crate::window(parent), PARENT);
    assert_ne!(crate::window(child), crate::window(parent));
}

#[test]
fn the_native_adapter_without_a_range_inherits_the_parent() {
    let parent = PieToken::mint(2);
    crate::seed(parent, PARENT);
    let child = NativePieEffects
        .derive(derivation(parent, GrantRange::Inherit))
        .expect("the stand-in accepts the call");
    assert_eq!(
        crate::calls(),
        vec![Call::Derive(parent, GrantRange::Inherit)],
        "inheriting derivation still goes through `derive`"
    );
    assert_eq!(
        crate::window(child),
        PARENT,
        "inheriting derivation keeps the parent's range"
    );
}

#[test]
fn cross_task_delivery_uses_accord_and_inherits_the_parent() {
    let parent = PieToken::mint(3);
    crate::seed(parent, PARENT);
    let child = NativePieEffects
        .accord(Accord {
            token: parent,
            target: TaskId::new(8),
            permission: Permission::FETCH,
            mark: Mark::of("delivered"),
        })
        .expect("the stand-in accepts the call");
    assert_eq!(crate::calls(), vec![Call::Accord(parent)]);
    assert_eq!(crate::window(child), PARENT);
}
