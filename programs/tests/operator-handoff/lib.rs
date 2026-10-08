#![allow(dead_code)]
extern crate env as abi;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;
pub use abi::{Mark, PieToken, TaskId};
use std::cell::RefCell;
thread_local! { static ENTRIES: RefCell<Vec<(PieToken, TaskId, TaskId, Mark, bool)>> = const { RefCell::new(Vec::new()) }; }
pub mod raw {
    use super::*;
    pub fn alive(token: PieToken) -> bool {
        ENTRIES.with(|entries| {
            entries
                .borrow()
                .iter()
                .any(|entry| entry.0 == token && entry.4)
        })
    }
    pub fn reserve(token: PieToken) -> Result<(TaskId, TaskId, Mark), ()> {
        ENTRIES.with(|entries| {
            entries
                .borrow()
                .iter()
                .find(|entry| entry.0 == token)
                .map(|entry| (entry.1, entry.2, entry.3))
                .ok_or(())
        })
    }
}
pub mod session {
    pub mod establish {
        #[derive(Debug, PartialEq, Eq)]
        pub enum DiscoveryFail {
            Missing,
            Ambiguous,
        }
        pub fn find(_: crate::TaskId, _: crate::Mark) -> Result<crate::PieToken, DiscoveryFail> {
            panic!("explicit reply import must not scan")
        }
        pub fn marked_as(token: crate::PieToken) -> Option<crate::Mark> {
            crate::raw::reserve(token).ok().map(|entry| entry.2)
        }
    }
}
#[path = "../../src/system/operator/service/claim.rs"]
mod claim;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_token_selects_the_handoff_among_same_role_instances() {
        let control = TaskId::new(1);
        let who = TaskId::new(2);
        let mark = system_api::operator::LINK_MARK;
        ENTRIES.with(|entries| {
            *entries.borrow_mut() = vec![
                (PieToken::mint(10), control, who, mark, true),
                (PieToken::mint(11), control, who, mark, true),
            ]
        });
        assert!(claim::valid_reply(who, PieToken::mint(11), control));
        ENTRIES.with(|entries| entries.borrow_mut().reverse());
        assert!(claim::valid_reply(who, PieToken::mint(11), control));
    }
    #[test]
    fn handoff_requires_live_capability_owner_giver_and_role() {
        let control = TaskId::new(1);
        let who = TaskId::new(2);
        let other = TaskId::new(3);
        let token = PieToken::mint(10);
        let mark = system_api::operator::LINK_MARK;
        for entry in [
            (token, other, who, mark, true),
            (token, control, other, mark, true),
            (token, control, who, Mark::NONE, true),
            (token, control, who, mark, false),
        ] {
            ENTRIES.with(|entries| *entries.borrow_mut() = vec![entry]);
            assert!(!claim::valid_reply(who, token, control));
        }
        ENTRIES.with(|entries| entries.borrow_mut().clear());
        assert!(!claim::valid_reply(who, token, control));
    }

    #[test]
    fn operator_request_dispatch_does_not_select_a_grant_from_the_session() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/system/operator/service");
        let session = std::fs::read_to_string(root.join("session.rs")).unwrap();
        assert!(session.contains("pub(super) struct Incoming"));
        assert!(!session.contains("grant:"));
        for file in ["session.rs", "run.rs", "claim.rs"] {
            let source = std::fs::read_to_string(root.join(file)).unwrap();
            assert!(
                !source.contains("grant_of"),
                "{file} selects an operation from marks"
            );
            assert!(
                !source.contains("Grant::MARKS"),
                "{file} dispatches through Grant::MARKS"
            );
        }
    }
}
