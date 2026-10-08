#![allow(dead_code)]
extern crate alloc;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;
pub use abi::{MailFail, Mark, Permission, PieToken, TaskId, Wait};
pub type PieResult<T> = Result<T, ()>;
use std::{cell::RefCell, collections::HashMap};
use wire::Message;
#[derive(Default)]
struct Native {
    next: usize,
    roots: HashMap<PieToken, (bool, bool)>,
    remote: HashMap<PieToken, PieToken>,
    seals: Vec<PieToken>,
    releases: Vec<PieToken>,
    allocation_fail: bool,
    ship_fail: bool,
    call_fail: bool,
    reply: Vec<u8>,
    calls: usize,
}
thread_local! { static NATIVE: RefCell<Native> = RefCell::default(); }
pub mod pie {
    use super::*;
    pub fn unseal_hole(mark: Mark) -> PieResult<PieToken> {
        assert_eq!(mark, operator::WATCH_MARK);
        NATIVE.with(|cell| {
            let mut n = cell.borrow_mut();
            if n.allocation_fail {
                return Err(());
            }
            n.next += 1;
            let token = PieToken::mint(n.next);
            n.roots.insert(token, (true, false));
            Ok(token)
        })
    }
    pub fn seal(token: PieToken) -> PieResult<()> {
        NATIVE.with(|cell| {
            let mut n = cell.borrow_mut();
            n.roots.get_mut(&token).unwrap().0 = false;
            n.seals.push(token);
        });
        Ok(())
    }
    pub fn release(token: PieToken) -> PieResult<()> {
        NATIVE.with(|cell| {
            let mut n = cell.borrow_mut();
            let root = n.roots.get_mut(&token).unwrap();
            assert!(!root.1);
            root.1 = true;
            n.releases.push(token);
        });
        Ok(())
    }
    pub fn accord(_: PieToken, _: TaskId, _: Permission, _: Mark) -> PieResult<PieToken> {
        unreachable!()
    }
    pub fn revoke(_: TaskId, _: PieToken) -> PieResult<()> {
        unreachable!()
    }
}
#[path = "../../../crates/resource/src/capability.rs"]
mod capability;
pub mod raw {
    pub use crate::capability::Capability;
}
pub mod port {
    use super::*;
    pub use abi::{Access, Policy};
    pub struct To(PieToken);
    impl To {
        pub fn seed(self) -> PieToken {
            self.0
        }
    }
    pub fn ship(source: PieToken, peer: TaskId, access: Access, policy: Policy) -> PieResult<To> {
        assert_eq!(peer, TaskId::new(7));
        assert_eq!(access.bits(), Access::STORE.bits());
        assert_eq!(policy.bits(), Policy::NONE.bits());
        NATIVE.with(|cell| {
            let mut n = cell.borrow_mut();
            if n.ship_fail {
                return Err(());
            }
            n.next += 1;
            let remote = PieToken::mint(n.next);
            n.remote.insert(remote, source);
            Ok(To(remote))
        })
    }
}
pub mod hand {
    use super::*;
    pub enum RecvFail {
        Mail(MailFail),
    }
    pub enum SourceFail {
        Receive(RecvFail),
    }
    pub struct Receiver<M>(PieToken, core::marker::PhantomData<M>);
    impl<M: Message> Receiver<M> {
        pub fn from_raw(token: PieToken) -> Self {
            Self(token, core::marker::PhantomData)
        }
        pub fn recv_from(&mut self, _: TaskId, _: &mut [u8], _: Wait) -> Result<M::In, SourceFail> {
            Err(SourceFail::Receive(RecvFail::Mail(MailFail::Busy)))
        }
    }
}
pub mod time {
    pub fn deadline(wait: crate::Wait) -> crate::Wait {
        wait
    }
    pub fn remain(wait: crate::Wait) -> crate::Wait {
        wait
    }
}
pub mod debug {
    pub fn put(_: &str) {}
}
pub mod operator {
    pub use system_api::operator::*;
    pub struct Face;
    impl Face {
        pub fn host(&self) -> crate::TaskId {
            crate::TaskId::new(7)
        }
        pub(crate) fn call(&self, request: Req, _: crate::Wait) -> Result<Said, Fail> {
            let Req::Watch { hole, road } = request else {
                panic!("unexpected request")
            };
            assert_eq!(road.as_str(), "svc");
            crate::NATIVE.with(|cell| {
                let mut n = cell.borrow_mut();
                assert!(n.remote.contains_key(&hole));
                n.calls += 1;
                if n.call_fail {
                    return Err(Fail::Unknown);
                }
                <Union as wire::Message>::fetch(&n.reply).ok_or(Fail::Unknown)
            })
        }
        pub fn tile(&self, _: &path::Path, _: crate::Wait) -> Result<Tile, Fail> {
            unreachable!()
        }
    }
    pub struct Tile;
    impl Tile {
        pub fn id(&self) -> EntryId {
            EntryId::new(0)
        }
    }
    pub use crate::production_watch::Watch;
}
pub use operator::Face;
mod face {
    pub fn map_code(code: u8) -> crate::operator::Fail {
        crate::operator::code_to_fail(code).unwrap_or(crate::operator::Fail::Unknown)
    }
}
#[path = "../../src/system/client/src/operator/watch.rs"]
mod production_watch;
#[cfg(test)]
mod tests {
    use super::*;
    use operator::{DENIED, Face, OK, Watch, path::Path};
    fn reset() {
        NATIVE.with(|n| {
            *n.borrow_mut() = Native {
                reply: vec![OK],
                ..Default::default()
            }
        });
    }
    fn assert_closed() {
        NATIVE.with(|n| {
            let n = n.borrow();
            assert_eq!(n.seals.len(), 1);
            assert_eq!(n.releases, n.seals);
            assert!(n.roots.values().all(|root| *root == (false, true)));
            assert!(n.remote.values().all(|root| !n.roots[root].0));
        });
    }
    #[test]
    fn successful_watch_owns_only_its_event_endpoint_and_drop_closes_remote_delivery() {
        reset();
        let face = Face;
        let watch = Watch::of(&face, Path::new("/svc"), Wait::POLL).unwrap();
        assert_eq!(watch.road(), "svc");
        NATIVE.with(|n| {
            let n = n.borrow();
            assert_eq!(n.calls, 1);
            assert!(n.seals.is_empty());
            assert!(n.releases.is_empty());
        });
        drop(watch);
        assert_closed();
        assert_eq!(face.host(), TaskId::new(7));
    }
    #[test]
    fn full_constructor_failures_seal_and_release_the_new_root() {
        for stage in 0..5 {
            reset();
            NATIVE.with(|cell| {
                let mut n = cell.borrow_mut();
                match stage {
                    0 => n.ship_fail = true,
                    1 => n.call_fail = true,
                    2 => n.reply = vec![DENIED],
                    3 => n.reply = vec![OK, 4],
                    _ => n.reply = vec![255],
                }
            });
            let face = Face;
            assert!(Watch::of(&face, Path::new("/svc"), Wait::POLL).is_err());
            assert_closed();
            NATIVE.with(|n| assert_eq!(n.borrow().calls, usize::from(stage != 0)));
        }
    }
    #[test]
    fn allocation_failure_does_not_ship_or_call_or_release_a_borrowed_endpoint() {
        reset();
        NATIVE.with(|n| n.borrow_mut().allocation_fail = true);
        assert!(Watch::of(&Face, Path::new("/svc"), Wait::POLL).is_err());
        NATIVE.with(|n| {
            let n = n.borrow();
            assert_eq!(n.calls, 0);
            assert!(n.remote.is_empty());
            assert!(n.releases.is_empty());
        });
    }
}
