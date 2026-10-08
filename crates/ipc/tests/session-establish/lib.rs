#![allow(dead_code)]
extern crate self as env;
extern crate self as resource;
extern crate self as wire;

use std::cell::RefCell;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    AtMost(usize),
    Forever,
}
impl Wait {
    pub const POLL: Self = Self::AtMost(0);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PieToken(pub usize);
impl PieToken {
    pub const NONE: Self = Self(0);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(pub usize);
impl TaskId {
    pub const fn new(x: usize) -> Self {
        Self(x)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permission(u8);
impl Permission {
    pub const FETCH: Self = Self(1);
    pub const STORE: Self = Self(2);
    pub const VEST: Self = Self(4);
}
impl std::ops::BitOr for Permission {
    type Output = Self;
    fn bitor(self, r: Self) -> Self {
        Self(self.0 | r.0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieError;
pub type PieResult<T> = Result<T, PieError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailFail {
    Busy,
    Dead,
}
pub type MailResult<T> = Result<T, MailFail>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleDir {
    Pull,
    Push,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtAddr(usize);
impl VirtAddr {
    pub fn new(x: usize) -> Self {
        Self(x)
    }
}

pub mod chrono {
    pub fn clock() -> u64 {
        super::fake::NOW.with(|x| *x.borrow())
    }
}
pub mod unit {
    use super::{TaskId, Wait};
    pub fn self_id() -> TaskId {
        TaskId(1)
    }
    pub fn fall(wait: Wait) -> Result<(), ()> {
        super::fake::FALLS.with(|x| *x.borrow_mut() += 1);
        if let Some(f) = super::fake::ON_FALL.with(|x| x.borrow_mut().take()) {
            f();
        }
        if let Wait::AtMost(n) = wait {
            super::fake::NOW.with(|x| *x.borrow_mut() += ((n as u64).max(1)) * 1_000_000);
        }
        Ok(())
    }
}
trait Flag {
    fn bits(self) -> u8;
}
#[derive(Clone, Copy)]
pub struct Access(u8);
impl Access {
    pub const FETCH: Self = Self(1);
    pub const STORE: Self = Self(2);
}
impl std::ops::BitOr for Access {
    type Output = Self;
    fn bitor(self, r: Self) -> Self {
        Self(self.0 | r.0)
    }
}
#[derive(Clone, Copy)]
pub struct Policy(u8);
impl Policy {
    pub const NONE: Self = Self(0);
    pub const VEST: Self = Self(1);
}

pub mod fake {
    use super::*;
    #[derive(Clone, Copy)]
    pub struct Candidate {
        pub token: PieToken,
        pub alive: bool,
        pub hole: bool,
        pub query: Option<(TaskId, TaskId, Mark)>,
    }
    thread_local! {
      pub static CANDIDATES:RefCell<Vec<Candidate>>=const{RefCell::new(Vec::new())};
      pub static OWN:RefCell<TaskId>=const{RefCell::new(TaskId(1))};
      pub static NEXT:RefCell<usize>=const{RefCell::new(100)};
      pub static NOW:RefCell<u64>=const{RefCell::new(0)};
      pub static FALLS:RefCell<usize>=const{RefCell::new(0)};
      pub static ON_FALL:RefCell<Option<Box<dyn FnOnce()>>>=const{RefCell::new(None)};
      pub static CLEAN:RefCell<Vec<(TaskId,PieToken)>>=const{RefCell::new(Vec::new())};
      pub static RELEASE:RefCell<Vec<PieToken>>=const{RefCell::new(Vec::new())};
      pub static SHIP_FAIL:RefCell<bool>=const{RefCell::new(false)};
      pub static UNSEAL_FAIL:RefCell<bool>=const{RefCell::new(false)};
      pub static SHIPS:RefCell<Vec<(PieToken,TaskId)>>=const{RefCell::new(Vec::new())};
      pub static SCANS:RefCell<usize>=const{RefCell::new(0)};
    }
    pub fn reset() {
        CANDIDATES.with(|x| x.borrow_mut().clear());
        OWN.with(|x| *x.borrow_mut() = TaskId(1));
        NEXT.with(|x| *x.borrow_mut() = 100);
        NOW.with(|x| *x.borrow_mut() = 0);
        FALLS.with(|x| *x.borrow_mut() = 0);
        ON_FALL.with(|x| *x.borrow_mut() = None);
        CLEAN.with(|x| x.borrow_mut().clear());
        RELEASE.with(|x| x.borrow_mut().clear());
        SHIP_FAIL.with(|x| *x.borrow_mut() = false);
        UNSEAL_FAIL.with(|x| *x.borrow_mut() = false);
        SHIPS.with(|x| x.borrow_mut().clear());
        SCANS.with(|x| *x.borrow_mut() = 0);
    }
    pub fn add(token: usize, alive: bool, owner: usize, mark: usize) {
        add_resource(token, alive, true, owner, mark);
    }
    pub fn add_resource(token: usize, alive: bool, hole: bool, owner: usize, mark: usize) {
        CANDIDATES.with(|x| {
            x.borrow_mut().push(Candidate {
                token: PieToken(token),
                alive,
                hole,
                query: Some((TaskId(1), TaskId(owner), Mark(mark))),
            })
        });
    }
    pub fn falls() -> usize {
        FALLS.with(|x| *x.borrow())
    }
    pub fn scans() -> usize {
        SCANS.with(|x| *x.borrow())
    }
    pub fn clean() -> Vec<(TaskId, PieToken)> {
        CLEAN.with(|x| x.borrow().clone())
    }
    pub fn released() -> Vec<PieToken> {
        RELEASE.with(|x| x.borrow().clone())
    }
}
pub mod raw {
    use super::*;
    pub struct Pies {
        i: usize,
    }
    impl Iterator for Pies {
        type Item = Pie;
        fn next(&mut self) -> Option<Pie> {
            let c = fake::CANDIDATES.with(|x| x.borrow().get(self.i).copied());
            self.i += 1;
            c.map(|c| Pie {
                token: c.token,
                owner: c.query.map_or(TaskId(0), |x| x.1),
                mark: c.query.map_or(Mark(0), |x| x.2),
            })
        }
    }
    pub struct Pie {
        pub token: PieToken,
        pub owner: TaskId,
        pub mark: Mark,
    }
    pub struct Hole(PieToken);
    impl Hole {
        pub fn from_raw(t: PieToken) -> Self {
            Self(t)
        }
        pub fn pull(&self, _: &mut [u8], _: Wait) -> Result<(usize, TaskId), PullError> {
            Err(PullError)
        }
        pub fn push(&self, _: &[u8], _: Wait) -> Result<(), PushError> {
            Err(PushError)
        }
    }
    pub struct PullError;
    pub struct PushError;
    pub fn pies() -> Pies {
        fake::SCANS.with(|x| *x.borrow_mut() += 1);
        Pies { i: 0 }
    }
    pub fn alive(t: PieToken) -> bool {
        fake::CANDIDATES.with(|x| {
            x.borrow()
                .iter()
                .find(|c| c.token == t)
                .is_some_and(|c| c.alive)
        })
    }
    pub fn reserve(t: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
        fake::CANDIDATES.with(|x| {
            x.borrow()
                .iter()
                .find(|c| c.token == t && c.hole)
                .and_then(|c| c.query)
                .ok_or(PieError)
        })
    }
    pub fn inspect(t: PieToken) -> PieResult<(TaskId, TaskId, Mark)> {
        fake::CANDIDATES.with(|x| {
            x.borrow()
                .iter()
                .find(|c| c.token == t)
                .and_then(|c| c.query)
                .ok_or(PieError)
        })
    }
}
pub mod pie {
    use super::*;
    pub fn release(t: PieToken) -> PieResult<()> {
        fake::RELEASE.with(|x| x.borrow_mut().push(t));
        Ok(())
    }
    pub fn revoke(peer: TaskId, t: PieToken) -> PieResult<()> {
        fake::CLEAN.with(|x| x.borrow_mut().push((peer, t)));
        Ok(())
    }
    pub fn unseal_hole(mark: Mark) -> PieResult<PieToken> {
        if fake::UNSEAL_FAIL.with(|x| x.replace(false)) {
            return Err(PieError);
        }
        let t = fake::NEXT.with(|x| {
            let n = *x.borrow();
            *x.borrow_mut() += 1;
            PieToken(n)
        });
        fake::CANDIDATES.with(|x| {
            x.borrow_mut().push(fake::Candidate {
                token: t,
                alive: true,
                hole: true,
                query: Some((TaskId(1), TaskId(0), mark)),
            })
        });
        Ok(t)
    }
    pub fn narrow(_: PieToken, _: Permission) -> PieResult<()> {
        Ok(())
    }
}
pub mod port {
    use super::*;
    #[derive(Clone, Copy)]
    pub struct Access(u8);
    impl Access {
        pub const FETCH: Self = Self(1);
        pub const STORE: Self = Self(2);
    }
    impl std::ops::BitOr for Access {
        type Output = Self;
        fn bitor(self, r: Self) -> Self {
            Self(self.0 | r.0)
        }
    }
    #[derive(Clone, Copy)]
    pub struct Policy(u8);
    impl Policy {
        pub const NONE: Self = Self(0);
        pub const VEST: Self = Self(1);
    }
    pub struct To(PieToken);
    impl To {
        pub fn seed(&self) -> PieToken {
            self.0
        }
    }
    pub fn ship(t: PieToken, to: TaskId, _: Access, _: Policy) -> Result<To, ()> {
        if fake::SHIP_FAIL.with(|x| x.replace(false)) {
            return Err(());
        }
        let seed = fake::NEXT.with(|x| {
            let n = *x.borrow();
            *x.borrow_mut() += 1;
            PieToken(n)
        });
        fake::SHIPS.with(|x| x.borrow_mut().push((t, to)));
        Ok(To(seed))
    }
}
pub trait Message {
    type In;
    type Buf: AsRef<[u8]> + AsMut<[u8]>;
    const EMPTY: Self::Buf;
    fn store(&self, out: &mut [u8]) -> Option<usize>;
    fn fetch(bytes: &[u8]) -> Option<Self::In>;
}
pub mod hand {
    use super::*;
    pub struct Receiver<M: Message>(PieToken, std::marker::PhantomData<M>);
    impl<M: Message> Receiver<M> {
        pub fn from_raw(t: PieToken) -> Self {
            Self(t, std::marker::PhantomData)
        }
    }
    pub struct Sender<M: Message>(PieToken, std::marker::PhantomData<M>);
    impl<M: Message> Sender<M> {
        pub fn from_raw(t: PieToken) -> Self {
            Self(t, std::marker::PhantomData)
        }
    }
}
pub mod time {
    use super::Wait;
    pub fn deadline(w: Wait) -> u64 {
        match w {
            Wait::Forever => u64::MAX,
            Wait::AtMost(n) => super::chrono::clock().saturating_add(n as u64 * 1_000_000),
        }
    }
    pub fn remain(d: u64) -> Wait {
        if d == u64::MAX {
            Wait::Forever
        } else {
            let n = d.saturating_sub(super::chrono::clock()) / 1_000_000;
            Wait::AtMost(n as usize)
        }
    }
}
pub mod session {
    #[path = "../../../src/session/establish.rs"]
    pub mod establish;
}

#[cfg(test)]
mod tests {
    use super::*;
    use session::establish::{self, DiscoveryFail, EstablishFail};
    fn matching(t: usize) {
        fake::add(t, true, 9, 4)
    }
    #[test]
    fn unique_result_is_independent_of_candidate_order_and_checks_reserve() {
        for order in [[2, 3], [3, 2]] {
            fake::reset();
            for t in order {
                if t == 2 {
                    fake::add(t, true, 9, 4)
                } else {
                    fake::add(t, true, 8, 4)
                }
            }
            assert_eq!(establish::find(TaskId(9), Mark(4)), Ok(PieToken(2)));
        }
    }
    #[test]
    fn duplicate_token_is_ambiguous_too() {
        fake::reset();
        matching(2);
        matching(2);
        assert_eq!(
            establish::find(TaskId(9), Mark(4)),
            Err(DiscoveryFail::Ambiguous)
        );
    }
    #[test]
    fn missing_waits_then_rechecks() {
        fake::reset();
        fake::ON_FALL.with(|x| *x.borrow_mut() = Some(Box::new(|| matching(5))));
        assert_eq!(
            establish::claim(TaskId(9), Mark(4), Wait::AtMost(8)),
            Ok(PieToken(5))
        );
        assert_eq!(fake::falls(), 1);
    }
    #[test]
    fn ambiguity_does_not_wait_or_cleanup_candidates() {
        fake::reset();
        matching(2);
        matching(3);
        assert_eq!(
            establish::claim(TaskId(9), Mark(4), Wait::AtMost(40)),
            Err(DiscoveryFail::Ambiguous)
        );
        assert_eq!(fake::falls(), 0);
        assert!(fake::released().is_empty());
        assert!(fake::clean().is_empty());
    }
    #[test]
    fn dead_and_uninspectable_resources_are_filtered() {
        fake::reset();
        fake::add(1, false, 9, 4);
        fake::CANDIDATES.with(|x| {
            x.borrow_mut().push(fake::Candidate {
                token: PieToken(2),
                alive: true,
                hole: false,
                query: None,
            })
        });
        fake::add(3, true, 8, 4);
        fake::add(4, true, 9, 7);
        matching(5);
        assert_eq!(establish::find(TaskId(9), Mark(4)), Ok(PieToken(5)));
    }
    #[test]
    fn endpoint_ambiguity_cleans_only_its_new_local_and_remote_caps() {
        fake::reset();
        matching(2);
        matching(3);
        assert_eq!(
            establish::endpoint(TaskId(9), Mark(4), Wait::AtMost(20)),
            Err(EstablishFail::Ambiguous)
        );
        let own = fake::SHIPS.with(|x| x.borrow()[0]);
        assert_eq!(fake::released(), vec![own.0]);
        assert_eq!(fake::clean(), vec![(TaskId(9), PieToken(101))]);
    }
    #[test]
    fn own_ship_failure_releases_new_local_hole() {
        fake::reset();
        fake::SHIP_FAIL.with(|x| *x.borrow_mut() = true);
        assert_eq!(
            establish::endpoint(TaskId(9), Mark(4), Wait::POLL),
            Err(EstablishFail::NoSeed)
        );
        assert_eq!(fake::released(), vec![PieToken(100)]);
        assert!(fake::clean().is_empty());
    }
    #[test]
    fn accept_rejects_dead_import_before_creating_or_shipping() {
        fake::reset();
        fake::add(7, false, 9, 4);
        assert_eq!(establish::accept(PieToken(7)), Err(EstablishFail::NoSeed));
        assert!(fake::SHIPS.with(|x| x.borrow().is_empty()));
        assert!(fake::released().is_empty());
        assert_eq!(fake::NEXT.with(|x| *x.borrow()), 100);
    }
    #[test]
    fn known_binding_is_validated_without_rescan_or_replacement() {
        fake::reset();
        matching(7);
        let mut endpoint = establish::accept(PieToken(7)).unwrap();
        let scans = fake::scans();
        matching(8);
        assert_eq!(endpoint.claim(TaskId(9), Mark(4), Wait::POLL), Ok(true));
        assert_eq!(endpoint.tx(), Some(PieToken(7)));
        assert_eq!(fake::scans(), scans);
    }
    #[test]
    fn dead_cached_binding_fails_without_falling_back_to_another_match() {
        fake::reset();
        matching(7);
        let mut endpoint = establish::accept(PieToken(7)).unwrap();
        fake::CANDIDATES.with(|x| {
            x.borrow_mut()
                .iter_mut()
                .find(|c| c.token == PieToken(7))
                .unwrap()
                .alive = false
        });
        matching(8);
        let scans = fake::scans();
        assert_eq!(
            endpoint.claim(TaskId(9), Mark(4), Wait::POLL),
            Err(DiscoveryFail::Missing)
        );
        assert_eq!(endpoint.tx(), Some(PieToken(7)));
        assert_eq!(fake::scans(), scans);
    }
    #[test]
    fn generic_discovery_finds_live_nonhole_but_endpoint_binding_rejects_it() {
        fake::reset();
        fake::add_resource(6, true, false, 9, 4);
        assert_eq!(establish::find(TaskId(9), Mark(4)), Ok(PieToken(6)));
        assert_eq!(establish::accept(PieToken(6)), Err(EstablishFail::NoSeed));
        assert!(fake::SHIPS.with(|x| x.borrow().is_empty()));
    }
    #[test]
    fn endpoint_rejects_nonhole_candidate_and_cleans_only_its_own_resources() {
        fake::reset();
        fake::add_resource(6, true, false, 9, 4);
        assert_eq!(
            establish::endpoint(TaskId(9), Mark(4), Wait::POLL),
            Err(EstablishFail::NoSeed)
        );
        assert_eq!(fake::released(), vec![PieToken(100)]);
        assert_eq!(fake::clean(), vec![(TaskId(9), PieToken(101))]);
        assert!(raw::alive(PieToken(6)));
    }
    #[test]
    fn owner_zero_generic_resource_is_discoverable() {
        fake::reset();
        fake::add_resource(6, true, false, 0, 4);
        assert_eq!(establish::find(TaskId(0), Mark(4)), Ok(PieToken(6)));
    }
    #[test]
    fn lend_ships_without_scanning_or_binding_existing_peer_endpoints() {
        fake::reset();
        matching(2);
        matching(3);
        let endpoint = establish::lend(TaskId(9), Mark(4)).unwrap();
        assert_eq!(endpoint.tx(), None);
        assert_eq!(endpoint.seed(), PieToken(101));
        assert_eq!(fake::scans(), 0);
        assert_eq!(fake::SHIPS.with(|x| x.borrow().len()), 1);
    }
    #[test]
    fn lend_ship_failure_releases_only_its_new_local_hole() {
        fake::reset();
        fake::SHIP_FAIL.with(|x| *x.borrow_mut() = true);
        assert_eq!(
            establish::lend(TaskId(9), Mark(4)),
            Err(EstablishFail::NoSeed)
        );
        assert_eq!(fake::released(), vec![PieToken(100)]);
        assert_eq!(fake::scans(), 0);
    }
}
