//! The real `NativePieEffects` dispatch driven against a host stand-in for `env::pie`.
//!
//! `supply/lease.rs` is compiled through `#[path]`. Its neighbours are the real ones
//! (`resource::raw` and `system_api::supply`) plus the real `Derivation`; the single thing
//! replaced is the `env::pie` surface, so that *which operation* the adapter takes - and
//! *what range the derived token would open* - can be observed instead of ecalled.
//!
//! Why this exists: a local derived root must take `env::pie::derive` with exactly the
//! assigned page range, while a cross-task delivery takes plain `env::pie::accord` and
//! inherits the parent's range. The parent is deliberately *not* pre-narrowed here.
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

pub use abi::{GrantRange, Mark, Permission, PieInfo, PieKind, PieResult, PieToken, TaskId};

#[path = "../../src/system/supply/derivation.rs"]
mod derivation;

use std::cell::RefCell;
use std::collections::HashMap;

/// One call the adapter made, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    /// Cross-task delivery: the child keeps the parent's window.
    Accord(PieToken),
    /// A local derivation with exactly this page range.
    Derive(PieToken, GrantRange),
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
    use crate::{GrantRange, Mark, Permission, PieResult, PieToken};

    /// Cross-task delivery: no separate range, the child keeps the parent's.
    pub fn accord(
        source: PieToken,
        _target: crate::TaskId,
        _permission: Permission,
        _mark: Mark,
    ) -> PieResult<PieToken> {
        STATE.with(|state| state.borrow_mut().calls.push(Call::Accord(source)));
        Ok(issue(source, None))
    }

    /// Local derivation: exactly the requested range, or the parent's when inheriting.
    pub fn derive(
        source: PieToken,
        _permission: Permission,
        _mark: Mark,
        range: GrantRange,
    ) -> PieResult<PieToken> {
        let window = match range {
            GrantRange::Inherit => None,
            GrantRange::Pages { offset, count } => Some((offset, count)),
        };
        STATE.with(|state| {
            state.borrow_mut().calls.push(Call::Derive(source, range));
        });
        Ok(issue(source, window))
    }
}

#[path = "../../src/system/supply/lease.rs"]
mod lease;

#[cfg(test)]
mod tests;
