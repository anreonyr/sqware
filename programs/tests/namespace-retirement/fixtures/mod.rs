//! Fixtures shared by the namespace retirement cases.

pub mod capability;
pub mod system;

use alloc::vec;
use env::{PieToken, TaskId, Wait};
use system_api::operator::{Binding, Entry, EntryId, EntryTarget, GrantId};
use system_client::operator::GrantInfo;

use crate::system::app::namespace::Installed;

const OPERATOR: TaskId = TaskId::new(7);

pub fn entry(id: usize) -> Entry {
    Entry {
        operator: OPERATOR,
        target: EntryTarget::Node(EntryId::new(id)),
    }
}

pub fn receipt(anchor: Entry, grant: GrantId) -> Installed {
    Installed {
        anchor,
        authority: Some(GrantInfo {
            id: grant,
            entry: anchor,
            entrance: PieToken::NONE,
        }),
        grant: Some(grant),
        bindings: vec![Binding {
            entry: entry(40),
            revision: 1,
        }],
        entries: vec![entry(41), entry(42)],
        wait: Wait::POLL,
    }
}
