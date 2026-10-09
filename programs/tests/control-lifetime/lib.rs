#![allow(dead_code)]
extern crate alloc;
extern crate env as abi;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;
pub use abi::{Mark, PieToken, TaskId, Wait};
use std::{cell::RefCell, collections::VecDeque};
#[derive(Default)]
struct Effects {
    created: u64,
    fail_at: Option<u64>,
    released: Vec<u64>,
    shut: Vec<u64>,
    joins: VecDeque<Result<bool, ()>>,
    waits: Vec<Wait>,
}
thread_local! { static EFFECTS: RefCell<Effects> = RefCell::new(Effects::default()); }
pub mod raw {
    pub fn reserve(
        token: crate::PieToken,
    ) -> Result<(crate::TaskId, crate::TaskId, crate::Mark), ()> {
        Ok((
            crate::TaskId::new(9),
            crate::TaskId::new(9),
            crate::Mark::of(if token.get() % 2 == 1 {
                "supply"
            } else {
                "ready"
            }),
        ))
    }
}
pub mod pie {
    pub fn shut(token: crate::PieToken) -> Result<(), ()> {
        crate::EFFECTS.with(|e| e.borrow_mut().shut.push(token.get() as u64));
        Ok(())
    }
    pub fn release(token: crate::PieToken) -> Result<(), ()> {
        crate::EFFECTS.with(|e| e.borrow_mut().released.push(token.get() as u64));
        Ok(())
    }
}
pub mod unit {
    pub struct Setup;
    impl Setup {
        pub fn channel(&self) -> &'static str {
            "supply"
        }
        pub fn ready(&self) -> Option<&'static str> {
            Some("ready")
        }
    }
    pub struct UnitFile;
    impl UnitFile {
        pub fn valid(&self) -> bool {
            true
        }
        pub fn supply(&self) -> &[Setup] {
            &[Setup]
        }
    }
    pub fn join(_: crate::TaskId, wait: crate::Wait) -> Result<bool, ()> {
        crate::EFFECTS.with(|e| {
            let mut e = e.borrow_mut();
            e.waits.push(wait);
            e.joins.pop_front().expect("unexpected join")
        })
    }
}
pub mod session {
    pub struct Endpoint {
        rx: crate::PieToken,
    }
    impl Endpoint {
        pub fn rx(&self) -> crate::PieToken {
            self.rx
        }
        pub fn tx(&self) -> Option<crate::PieToken> {
            Some(self.rx)
        }
        pub fn claim(
            &mut self,
            _: crate::TaskId,
            _: crate::Mark,
            _: crate::Wait,
        ) -> Result<bool, establish::DiscoveryFail> {
            Ok(true)
        }
    }
    pub mod establish {
        pub enum DiscoveryFail {
            Missing,
            Ambiguous,
        }
        pub fn endpoint(
            _: crate::TaskId,
            _: crate::Mark,
            _: crate::Wait,
        ) -> Result<super::Endpoint, ()> {
            crate::EFFECTS.with(|e| {
                let mut e = e.borrow_mut();
                e.created += 1;
                if e.fail_at == Some(e.created) {
                    Err(())
                } else {
                    Ok(super::Endpoint {
                        rx: crate::PieToken::mint(e.created as usize),
                    })
                }
            })
        }
    }
}
mod control {
    pub mod table {
        #[derive(PartialEq)]
        pub enum State {
            Ready,
        }
        pub struct Row {
            pub state: State,
        }
        pub struct Table;
        impl Table {
            pub fn find(&self, _: &str) -> Option<Row> {
                Some(Row {
                    state: State::Ready,
                })
            }
        }
    }
    pub mod start {
        #[derive(Debug, PartialEq)]
        pub enum Error {
            Step(&'static str),
        }
    }
    pub mod verdict {
        #[derive(Debug, PartialEq)]
        pub enum Fail {
            Unknown,
        }
        #[derive(Debug, PartialEq)]
        pub enum Reaped {
            Now,
            Waited,
            Unsettled,
        }
    }
    pub mod task {
        pub struct Readiness<'a> {
            pub name: &'a str,
        }
        pub fn ready(
            _: &mut super::table::Table,
            _: Readiness<'_>,
            channels: &mut [crate::session::Endpoint],
        ) -> Result<bool, super::verdict::Fail> {
            Ok(!channels.is_empty())
        }
    }
    mod service {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/system/control/unit/service.rs"
        ));
    }
    mod wait {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/system/control/unit/wait.rs"
        ));
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{EFFECTS, Effects, TaskId, Wait};
        fn reset() {
            EFFECTS.with(|e| *e.borrow_mut() = Effects::default());
        }
        #[test]
        fn successful_boot_releases_only_owned_channels() {
            reset();
            let mut s = service::Service::new(TaskId::new(9));
            s.connect(&crate::unit::UnitFile).unwrap();
            assert!(
                s.ready(&mut table::Table, task::Readiness { name: "boot" })
                    .unwrap()
            );
            assert_eq!(s.task(), TaskId::new(9));
            drop(s);
            EFFECTS.with(|e| assert_eq!(e.borrow().released, [1, 2]));
        }
        #[test]
        fn partial_failure_rolls_back_immediately_and_keeps_previous_channels() {
            reset();
            let mut s = service::Service::new(TaskId::new(9));
            s.connect(&crate::unit::UnitFile).unwrap();
            EFFECTS.with(|e| e.borrow_mut().fail_at = Some(4));
            assert!(s.connect(&crate::unit::UnitFile).is_err());
            EFFECTS.with(|e| assert_eq!(e.borrow().released, [3]));
            drop(s);
            EFFECTS.with(|e| assert_eq!(e.borrow().released, [3, 1, 2]));
        }
        #[test]
        fn reconnect_replaces_and_reclaims_previous_channels() {
            reset();
            let mut s = service::Service::new(TaskId::new(9));
            s.connect(&crate::unit::UnitFile).unwrap();
            s.connect(&crate::unit::UnitFile).unwrap();
            EFFECTS.with(|e| assert_eq!(e.borrow().released, [1, 2]));
            drop(s);
            EFFECTS.with(|e| assert_eq!(e.borrow().released, [1, 2, 3, 4]));
        }
        #[test]
        fn supplied_images_are_released_once_after_ready() {
            reset();
            let mut s = service::Service::new(TaskId::new(9));
            s.hold_supply(crate::PieToken::mint(41)).unwrap();
            s.connect(&crate::unit::UnitFile).unwrap();
            s.ready(&mut table::Table, task::Readiness { name: "boot" })
                .unwrap();
            EFFECTS.with(|e| {
                assert_eq!(e.borrow().shut, [41]);
                assert_eq!(e.borrow().released, [41]);
            });
            drop(s);
            EFFECTS.with(|e| assert_eq!(e.borrow().released, [41, 1, 2]));
        }
        #[test]
        fn failed_start_reclaims_image_supplies() {
            reset();
            let mut s = service::Service::new(TaskId::new(9));
            s.hold_supply(crate::PieToken::mint(41)).unwrap();
            drop(s);
            EFFECTS.with(|e| {
                assert_eq!(e.borrow().shut, [41]);
                assert_eq!(e.borrow().released, [41]);
            });
        }
        #[test]
        fn supply_selection_uses_declared_channel_not_first_endpoint() {
            reset();
            let mut s = service::Service::new(TaskId::new(9));
            s.connect(&crate::unit::UnitFile).unwrap();
            assert_eq!(
                s.claim_supply(crate::Mark::of("ready"), Wait::POLL)
                    .unwrap(),
                crate::PieToken::mint(2)
            );
            assert!(
                s.claim_supply(crate::Mark::of("absent"), Wait::POLL)
                    .is_err()
            );
        }
        fn reap(results: &[Result<bool, ()>], wait_for: Wait) -> verdict::Reaped {
            reset();
            EFFECTS.with(|e| e.borrow_mut().joins.extend(results));
            wait::until(TaskId::new(9), wait_for)
        }
        #[test]
        fn already_dead_and_poll_live_are_distinguished() {
            assert_eq!(reap(&[Ok(true)], Wait::POLL), verdict::Reaped::Now);
            assert_eq!(reap(&[Ok(false)], Wait::POLL), verdict::Reaped::Unsettled);
        }
        #[test]
        fn exit_during_wait_is_waited_and_live_after_deadline_is_unsettled() {
            assert_eq!(
                reap(&[Ok(false), Ok(true), Ok(true)], Wait::AtMost(2)),
                verdict::Reaped::Waited
            );
            assert_eq!(
                reap(&[Ok(false), Ok(false), Ok(false)], Wait::AtMost(2)),
                verdict::Reaped::Unsettled
            );
        }
        #[test]
        fn wait_error_does_not_claim_a_live_task_exited() {
            assert_eq!(
                reap(&[Ok(false), Err(()), Ok(false)], Wait::AtMost(2)),
                verdict::Reaped::Unsettled
            );
        }
    }
}
