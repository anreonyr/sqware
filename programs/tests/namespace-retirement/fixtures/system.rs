//! Fake System exposing the stable issuer root over the fake capability.

use core::cell::RefCell;
use system_api::operator::Entry;

use crate::fixtures::capability::Capability;

pub struct System {
    pub capability: Capability,
    pub root: Entry,
}

impl System {
    pub fn rooted(root: Entry) -> Self {
        Self {
            capability: Capability {
                root,
                revoke_states: RefCell::new(alloc::vec::Vec::new()),
                revoked: RefCell::new(alloc::vec::Vec::new()),
                unbind_states: RefCell::new(alloc::vec::Vec::new()),
                unbound: RefCell::new(alloc::vec::Vec::new()),
                unpart_states: RefCell::new(alloc::vec::Vec::new()),
                unparted: RefCell::new(alloc::vec::Vec::new()),
            },
            root,
        }
    }
}
