#![allow(dead_code)]
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;
use std::{cell::RefCell, collections::HashSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PieToken(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskId(pub usize);
#[derive(Clone, Copy)]
pub struct Mark(pub usize);
impl Mark {
    pub const NONE: Self = Self(0);
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Permission(u8);
impl std::ops::BitOr for Permission {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}
pub type PieResult<T> = Result<T, ()>;
pub mod port {
    pub struct Access(u8);
    impl Access {
        pub const FETCH: Self = Self(1);
        pub const STORE: Self = Self(2);
        pub fn bits(self) -> crate::Permission {
            crate::Permission(self.0)
        }
    }
    pub struct Policy;
    impl Policy {
        pub const VEST: Self = Self;
        pub fn bits(self) -> crate::Permission {
            crate::Permission(4)
        }
    }
}
pub mod session {
    #[derive(Debug)]
    pub enum CallFail {
        Busy,
        Closed,
        Send(()),
        Receive(()),
    }
}
#[derive(Default)]
struct Native {
    next: usize,
    live: HashSet<(TaskId, PieToken)>,
    revoked: Vec<(TaskId, PieToken)>,
    fail: bool,
}
thread_local! { static NATIVE: RefCell<Native> = RefCell::default(); }
pub mod pie {
    use super::*;
    pub fn accord(
        source: PieToken,
        peer: TaskId,
        permission: Permission,
        mark: Mark,
    ) -> PieResult<PieToken> {
        assert_eq!(source, PieToken(9));
        assert_eq!(permission, Permission(7));
        assert_eq!(mark.0, 0);
        NATIVE.with(|cell| {
            let mut n = cell.borrow_mut();
            if n.fail {
                return Err(());
            }
            n.next += 1;
            let remote = PieToken(n.next + 100);
            n.live.insert((peer, remote));
            Ok(remote)
        })
    }
    pub fn revoke(peer: TaskId, remote: PieToken) -> PieResult<()> {
        NATIVE.with(|cell| {
            let mut n = cell.borrow_mut();
            assert!(n.live.remove(&(peer, remote)));
            n.revoked.push((peer, remote));
        });
        Ok(())
    }
    pub fn unseal_hole(_: Mark) -> PieResult<PieToken> {
        unreachable!()
    }
    pub fn seal(_: PieToken) -> PieResult<()> {
        unreachable!()
    }
    pub fn release(_: PieToken) -> PieResult<()> {
        unreachable!()
    }
}
#[path = "../../../crates/resource/src/capability.rs"]
mod capability;
pub mod raw {
    pub use crate::capability::Loan;
}
#[path = "../../src/system/client/src/operator/handoff.rs"]
mod handoff;
#[cfg(test)]
mod tests {
    use super::*;
    use session::CallFail;
    fn reset() {
        NATIVE.with(|n| *n.borrow_mut() = Native::default());
    }
    #[test]
    fn failed_admission_reclaims_each_grant_without_accumulating_duplicates() {
        reset();
        for step in 0..30 {
            let result = handoff::offer(
                &PieToken(9),
                TaskId(7),
                |_| -> Result<(bool, ()), CallFail> {
                    Err(match step % 3 {
                        0 => CallFail::Busy,
                        1 => CallFail::Closed,
                        _ => CallFail::Send(()),
                    })
                },
            );
            assert!(result.is_err());
            NATIVE.with(|n| assert!(n.borrow().live.is_empty()));
        }
        NATIVE.with(|n| assert_eq!(n.borrow().revoked.len(), 30));
    }
    #[test]
    fn normal_rejection_reclaims_delivery_even_when_response_is_well_formed() {
        reset();
        assert_eq!(
            handoff::offer(&PieToken(9), TaskId(7), |_| Ok((false, 4))),
            Ok(4)
        );
        NATIVE.with(|n| assert!(n.borrow().live.is_empty()));
    }
    #[test]
    fn accepted_delivery_is_not_revoked() {
        reset();
        assert_eq!(
            handoff::offer(&PieToken(9), TaskId(7), |token| Ok((true, token))),
            Ok(PieToken(101))
        );
        NATIVE.with(|n| {
            let n = n.borrow();
            assert_eq!(n.live.len(), 1);
            assert!(n.revoked.is_empty());
        });
    }
    #[test]
    fn lost_reply_retains_delivery_because_server_admission_is_uncertain() {
        reset();
        let result = handoff::offer(
            &PieToken(9),
            TaskId(7),
            |_| -> Result<(bool, ()), CallFail> { Err(CallFail::Receive(())) },
        );
        assert!(result.is_err());
        NATIVE.with(|n| {
            let n = n.borrow();
            assert_eq!(n.live.len(), 1);
            assert!(n.revoked.is_empty());
        });
    }
    #[test]
    fn failed_grant_never_invokes_the_request() {
        reset();
        NATIVE.with(|n| n.borrow_mut().fail = true);
        let result = handoff::offer(
            &PieToken(9),
            TaskId(7),
            |_| -> Result<(bool, ()), CallFail> { panic!("request without delivery") },
        );
        assert!(result.is_err());
        NATIVE.with(|n| assert!(n.borrow().live.is_empty()));
    }
}
