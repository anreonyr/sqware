//! The real `NativePieEffects` dispatch driven against a host stand-in for `env::pie`.
//!
//! `supply/lease.rs` is compiled through `#[path]`. Its two neighbours are the real
//! ones (`resource::raw` and `system_api::supply`); the single thing replaced is the
//! `env::pie` surface, so that *which* accord the adapter takes - and *what range the
//! derived token would open* - can be observed instead of ecalled. `resource` is only
//! there so the two paths resolve: the adapter's `accord` never reaches it.
//!
//! Why this exists: in the product the manager hands `RootAssignment.window` a pair of
//! coordinates that its own parent is **already** narrowed to, so passing them is
//! redundant and a scenario that flips them to `None` still passes. The adapter's
//! `Some`-branch dispatch is therefore only load-bearing when the parent is *not*
//! pre-narrowed, which is what `tests.rs` sets up.
//!
//! Run it with the host target spelled out:
//!
//! ```text
//! cargo test --manifest-path programs/tests/lease-window/Cargo.toml --target x86_64-unknown-linux-gnu
//! ```
//!
//! A bare `cargo test` from this directory walks up to the repository's
//! `.cargo/config.toml` and inherits the riscv64 target.

#![allow(dead_code)]

extern crate alloc;
extern crate self as env;

pub use abi::{Mark, Permission, PieInfo, PieKind, PieResult, PieToken, TaskId};

use std::cell::RefCell;
use std::collections::HashMap;

/// One call the adapter made, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    /// Plain inheritance: the child keeps the parent's window.
    Accord(PieToken),
    /// An explicit window: `(offset, count)` pages from the parent's origin.
    Window(PieToken, usize, usize),
}

#[derive(Default)]
struct State {
    next: usize,
    /// token -> its effective window; what `Open` would hand back.
    windows: HashMap<usize, (usize, usize)>,
    calls: Vec<Call>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Give a token a window (the stand-in for the kernel's own bookkeeping).
pub fn seed(token: PieToken, window: (usize, usize)) {
    STATE.with(|state| {
        state.borrow_mut().windows.insert(token.get(), window);
    });
}

/// The window a token would open at.
pub fn window(token: PieToken) -> (usize, usize) {
    STATE.with(|state| {
        *state
            .borrow()
            .windows
            .get(&token.get())
            .expect("a token the fixture seeded or issued")
    })
}

/// Every call the adapter made, in order.
pub fn calls() -> Vec<Call> {
    STATE.with(|state| state.borrow().calls.clone())
}

/// Issue a child for `parent`; `window` = the range it opens at.
fn issue(parent: PieToken, window: Option<(usize, usize)>) -> PieToken {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let inherited = *state.windows.get(&parent.get()).expect("a seeded parent");
        state.next += 1;
        let child = PieToken::mint(10_000 + state.next);
        state
            .windows
            .insert(child.get(), window.unwrap_or(inherited));
        child
    })
}

pub mod pie {
    use super::{Call, STATE, issue};
    use crate::{Mark, Permission, PieResult, PieToken, TaskId};

    /// Inheritance: no window asked for, the child keeps the parent's.
    pub fn accord(
        source: PieToken,
        target: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> PieResult<PieToken> {
        let _ = (target, permission, mark);
        STATE.with(|state| state.borrow_mut().calls.push(Call::Accord(source)));
        Ok(issue(source, None))
    }

    /// An explicit window: exactly `(offset, count)` pages from the parent's origin.
    pub fn accord_window(
        source: PieToken,
        target: TaskId,
        permission: Permission,
        mark: Mark,
        offset: usize,
        count: usize,
    ) -> PieResult<PieToken> {
        let _ = (target, permission, mark);
        STATE.with(|state| {
            state
                .borrow_mut()
                .calls
                .push(Call::Window(source, offset, count));
        });
        Ok(issue(source, Some((offset, count))))
    }
}

#[path = "../../src/system/supply/lease.rs"]
mod lease;

#[cfg(test)]
mod tests;
