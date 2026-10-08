#![allow(dead_code)]
extern crate alloc;
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
        pub fn vested_by(token: crate::PieToken) -> Option<crate::TaskId> {
            crate::ENTRIES.with(|entries| {
                entries
                    .borrow()
                    .iter()
                    .find(|entry| entry.0 == token && entry.4)
                    .map(|entry| entry.1)
            })
        }
    }
}
#[path = "../../src/system/operator/service/claim.rs"]
mod claim;
#[path = "../../src/support/face/desk.rs"]
mod desk;
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
    fn explicit_request_requires_live_native_owner_and_ask_role() {
        let who = TaskId::new(2);
        let other = TaskId::new(3);
        let token = PieToken::mint(12);
        let ask = system_api::operator::ASK_MARK;
        ENTRIES.with(|entries| *entries.borrow_mut() = vec![(token, who, who, ask, true)]);
        assert!(claim::valid_request(who, token, who));
        for entry in [
            (token, other, who, ask, true),
            (token, who, other, ask, true),
            (token, who, who, Mark::NONE, true),
            (token, who, who, ask, false),
        ] {
            ENTRIES.with(|entries| *entries.borrow_mut() = vec![entry]);
            assert!(!claim::valid_request(who, token, who));
        }
        ENTRIES.with(|entries| entries.borrow_mut().clear());
        assert!(!claim::valid_request(who, token, who));
        ENTRIES.with(|entries| *entries.borrow_mut() = vec![(token, who, who, ask, true)]);
        assert!(!claim::valid_request(who, token, other));
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

    #[test]
    fn desk_keeps_two_sessions_for_one_task_and_exact_replay_is_idempotent() {
        use desk::{Desk, DeskFail};
        let who = TaskId::new(7);
        let reply_a = PieToken::mint(20);
        let ask_a = PieToken::mint(21);
        let reply_b = PieToken::mint(22);
        let ask_b = PieToken::mint(23);
        let mut desk = Desk::new();
        desk.admit(who, (reply_a, ask_a)).unwrap();
        assert_eq!(desk.admit(who, (reply_a, ask_a)), Err(DeskFail::Already));
        desk.admit(who, (reply_b, ask_b)).unwrap();
        assert_eq!(desk.occupied(), 2);
        assert_eq!(desk.guest(ask_a).unwrap().reply(), reply_a);
        assert_eq!(desk.guest(ask_b).unwrap().reply(), reply_b);
    }

    #[test]
    fn desk_rejects_shared_endpoint_conflicts_without_replacing_sessions() {
        use desk::{Desk, DeskFail};
        let who = TaskId::new(7);
        let reply = PieToken::mint(30);
        let ask = PieToken::mint(31);
        let mut desk = Desk::new();
        desk.admit(who, (reply, ask)).unwrap();
        assert_eq!(
            desk.admit(who, (reply, PieToken::mint(32))),
            Err(DeskFail::Conflict)
        );
        assert_eq!(
            desk.admit(who, (PieToken::mint(33), ask)),
            Err(DeskFail::Conflict)
        );
        assert_eq!(desk.occupied(), 1);
        assert_eq!(desk.guest(ask).unwrap().reply(), reply);
        assert!(desk.contains_reply(reply));
        assert!(!desk.contains_reply(PieToken::mint(33)));
    }

    #[test]
    fn desk_sweeps_only_the_dead_session_and_returns_its_pair() {
        use desk::Desk;
        let who = TaskId::new(7);
        let reply_a = PieToken::mint(40);
        let ask_a = PieToken::mint(41);
        let reply_b = PieToken::mint(42);
        let ask_b = PieToken::mint(43);
        let link = system_api::operator::LINK_MARK;
        ENTRIES.with(|entries| {
            *entries.borrow_mut() = vec![
                (reply_a, TaskId::new(1), who, link, false),
                (reply_b, TaskId::new(1), who, link, true),
                (ask_a, who, who, system_api::operator::ASK_MARK, true),
                (ask_b, who, who, system_api::operator::ASK_MARK, true),
            ]
        });
        let mut desk = Desk::new();
        desk.admit(who, (reply_a, ask_a)).unwrap();
        desk.admit(who, (reply_b, ask_b)).unwrap();
        let mut removed = Vec::new();
        assert_eq!(desk.sweep_each(|gone| removed.push(gone)), 1);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].reply, reply_a);
        assert_eq!(removed[0].ask, ask_a);
        assert_eq!(desk.occupied(), 1);
        assert_eq!(desk.guest(ask_b).unwrap().reply(), reply_b);
        ENTRIES.with(|entries| {
            entries
                .borrow_mut()
                .iter_mut()
                .find(|entry| entry.0 == ask_b)
                .unwrap()
                .4 = false;
        });
        let mut second_removed = Vec::new();
        assert_eq!(desk.sweep_each(|gone| second_removed.push(gone)), 1);
        assert_eq!(second_removed[0].reply, reply_b);
        assert_eq!(second_removed[0].ask, ask_b);
        assert_eq!(desk.occupied(), 0);
    }

    #[test]
    fn operator_outboxes_are_keyed_by_reply_capability() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/system/operator/service");
        let session = std::fs::read_to_string(root.join("session.rs")).unwrap();
        assert!(session.contains("pub reply: PieToken"));
        assert!(session.contains("position(|out| out.reply == reply_token)"));
        assert!(!session.contains("position(|out| out.who =="));
        assert!(session.contains("out.reply == gone.reply"));
        let tip = std::fs::read_to_string(root.join("tip.rs")).unwrap();
        assert!(tip.contains("if !desk.contains_reply(*reply)"));
        assert!(tip.contains("Err(DeskFail::Conflict) => return Ok(Progress::Done)"));
    }
}
