//! Fake Operator capability: revoke, unbind, and unpart only, each recorded.

use alloc::vec::Vec;
use core::cell::RefCell;
use env::Wait;
use system_api::operator::{self as api, Binding, Entry, GrantId, GrantState};

pub struct Capability {
    pub root: Entry,
    pub revoke_states: RefCell<Vec<Result<GrantState, api::Fail>>>,
    pub revoked: RefCell<Vec<(Entry, GrantId)>>,
    pub unbind_states: RefCell<Vec<Result<(), api::Fail>>>,
    pub unbound: RefCell<Vec<Binding>>,
    pub unpart_states: RefCell<Vec<Result<(), api::Fail>>>,
    pub unparted: RefCell<Vec<Entry>>,
}

impl Capability {
    pub fn revoke(&self, anchor: Entry, id: GrantId, _wait: Wait) -> Result<GrantState, api::Fail> {
        self.revoked.borrow_mut().push((anchor, id));
        let mut states = self.revoke_states.borrow_mut();
        if states.is_empty() {
            if anchor == self.root {
                Ok(GrantState::Closed)
            } else {
                Err(api::Fail::Denied)
            }
        } else {
            states.remove(0)
        }
    }

    pub fn unbind(&self, binding: Binding, _wait: Wait) -> Result<(), api::Fail> {
        self.unbound.borrow_mut().push(binding);
        let mut states = self.unbind_states.borrow_mut();
        if states.is_empty() {
            Ok(())
        } else {
            states.remove(0)
        }
    }

    pub fn unpart(&self, entry: Entry, _wait: Wait) -> Result<(), api::Fail> {
        self.unparted.borrow_mut().push(entry);
        let mut states = self.unpart_states.borrow_mut();
        if states.is_empty() {
            Ok(())
        } else {
            states.remove(0)
        }
    }
}
