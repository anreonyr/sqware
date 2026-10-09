#![allow(dead_code)]
extern crate alloc;
extern crate self as env;
extern crate self as resource;
pub use ::wire::{Contract, Message};

use std::{cell::RefCell, collections::VecDeque};
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
    pub const WIDTH: usize = 8;
    pub fn to_bytes(self) -> [u8; 8] {
        (self.0 as u64).to_le_bytes()
    }
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Some(Self(
            u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?) as usize
        ))
    }
    pub const fn get(self) -> usize {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskId(pub usize);
impl TaskId {
    pub const WIDTH: usize = 8;
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
impl MailFail {
    pub const fn code(self) -> isize {
        match self {
            Self::Busy => -1,
            Self::Dead => -2,
        }
    }
}
pub struct MailError {
    pub source: MailFail,
}
pub type MailResult<T> = Result<T, MailError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailCondition {
    Pull,
    Push,
    Empty,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtAddr(usize);
impl VirtAddr {
    pub fn new(x: usize) -> Self {
        Self(x)
    }
}

impl wire::Field for TaskId {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&(self.0 as u64).to_le_bytes())
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(Self(
            u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?) as usize
        ))
    }
}
impl wire::Field for PieToken {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes())
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Self::from_bytes(bytes)
    }
}
pub mod wire {
    pub use ::wire::{Contract, Field, Message};
}
pub mod debug {
    pub fn put(_: &str) {}
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
    #[derive(Clone)]
    pub struct Handshake {
        pub host: TaskId,
        pub transport: PieToken,
        pub bootstrap: Option<Vec<u8>>,
        pub bootstrap_sender: TaskId,
        pub ack: Vec<u8>,
        pub ack_sender: TaskId,
        pub advance_ms: usize,
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
      pub static NARROW_FAIL:RefCell<bool>=const{RefCell::new(false)};
      pub static UNSEAL_FAIL:RefCell<bool>=const{RefCell::new(false)};
      pub static SHIPS:RefCell<Vec<(PieToken,TaskId)>>=const{RefCell::new(Vec::new())};
      pub static SCANS:RefCell<usize>=const{RefCell::new(0)};
      pub static INBOX:RefCell<VecDeque<(PieToken,Vec<u8>,TaskId)>>=const{RefCell::new(VecDeque::new())};
      pub static HANDSHAKES:RefCell<VecDeque<Handshake>>=const{RefCell::new(VecDeque::new())};
      pub static ACTIVE:RefCell<Option<(PieToken,Handshake,bool)>>=const{RefCell::new(None)};
      pub static WAITS:RefCell<Vec<Wait>>=const{RefCell::new(Vec::new())};
      pub static PUSHES:RefCell<Vec<(PieToken,Vec<u8>)>>=const{RefCell::new(Vec::new())};
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
        NARROW_FAIL.with(|x| *x.borrow_mut() = false);
        SHIPS.with(|x| x.borrow_mut().clear());
        SCANS.with(|x| *x.borrow_mut() = 0);
        INBOX.with(|x| x.borrow_mut().clear());
        HANDSHAKES.with(|x| x.borrow_mut().clear());
        ACTIVE.with(|x| *x.borrow_mut() = None);
        WAITS.with(|x| x.borrow_mut().clear());
        PUSHES.with(|x| x.borrow_mut().clear());
    }
    pub fn add(token: usize, alive: bool, owner: usize, mark: usize) {
        add_resource(token, alive, true, owner, mark);
    }
    pub fn add_resource(token: usize, alive: bool, hole: bool, owner: usize, mark: usize) {
        add_transfer(token, alive, hole, 1, owner, mark);
    }
    pub fn add_transfer(
        token: usize,
        alive: bool,
        hole: bool,
        vestor: usize,
        owner: usize,
        mark: usize,
    ) {
        CANDIDATES.with(|x| {
            x.borrow_mut().push(Candidate {
                token: PieToken(token),
                alive,
                hole,
                query: Some((TaskId(vestor), TaskId(owner), Mark(mark))),
            })
        });
    }
    pub fn handshake(h: Handshake) {
        HANDSHAKES.with(|x| x.borrow_mut().push_back(h));
    }
    pub fn push_message(token: PieToken, bytes: Vec<u8>, from: TaskId) {
        INBOX.with(|x| x.borrow_mut().push_back((token, bytes, from)));
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
        pub fn pull(&self, buffer: &mut [u8], wait: Wait) -> Result<(usize, TaskId), PullError> {
            fake::WAITS.with(|x| x.borrow_mut().push(wait));
            let frame = fake::INBOX
                .with(|x| {
                    let mut q = x.borrow_mut();
                    let i = q.iter().position(|(t, _, _)| *t == self.0)?;
                    q.remove(i)
                })
                .ok_or(PullError {
                    source: MailFail::Busy,
                })?;
            if frame.1.len() > buffer.len() {
                return Err(PullError {
                    source: MailFail::Dead,
                });
            }
            buffer[..frame.1.len()].copy_from_slice(&frame.1);
            let advance = fake::ACTIVE.with(|x| {
                x.borrow()
                    .as_ref()
                    .and_then(|(rx, h, done)| (*rx == self.0 && !*done).then_some(h.advance_ms))
            });
            if let Some(ms) = advance {
                fake::NOW.with(|x| *x.borrow_mut() += (ms as u64) * 1_000_000);
                fake::ACTIVE.with(|x| {
                    if let Some((_, _, done)) = x.borrow_mut().as_mut() {
                        *done = true;
                    }
                });
            }
            Ok((frame.1.len(), frame.2))
        }
        pub fn push(&self, bytes: &[u8], _: Wait) -> Result<(), PushError> {
            fake::PUSHES.with(|x| x.borrow_mut().push((self.0, bytes.to_vec())));
            if let Some((rx, h, _)) = fake::ACTIVE
                .with(|x| x.borrow().clone())
                .filter(|(_, h, _)| h.transport == self.0)
            {
                fake::push_message(rx, h.ack, h.ack_sender);
            }
            let _ = bytes;
            Ok(())
        }
        pub fn depth(&self) -> Result<usize, PullError> {
            Ok(fake::INBOX.with(|x| x.borrow().iter().filter(|(t, _, _)| *t == self.0).count()))
        }
        pub fn wait(&self, _: MailCondition, _: Wait) -> MailResult<bool> {
            Ok(false)
        }
    }
    #[derive(Debug)]
    pub struct PullError {
        pub source: MailFail,
    }
    #[derive(Debug)]
    pub struct PushError {
        pub source: MailFail,
    }
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
    pub fn inspect(t: PieToken) -> PieResult<crate::PieInfo> {
        fake::CANDIDATES.with(|x| {
            x.borrow()
                .iter()
                .find(|c| c.token == t)
                .and_then(|c| c.query)
                .map(|(vestor, owner, mark)| crate::PieInfo { vestor, owner, mark, alive: true }).ok_or(PieError)
        })
    }
}
pub mod pie {
    pub fn unseal(args: crate::UnsealArgs) -> PieResult<PieToken> {
        match args { crate::UnsealArgs::Hole { mark } => hole_token(mark) }
    }

    use super::*;
    pub fn seal(_: PieToken) -> PieResult<()> {
        Ok(())
    }
    pub fn release(t: PieToken, _mode: crate::ReleaseMode) -> PieResult<()> {
        fake::RELEASE.with(|x| x.borrow_mut().push(t));
        Ok(())
    }
    pub fn revoke(peer: TaskId, t: PieToken) -> PieResult<()> {
        fake::CLEAN.with(|x| x.borrow_mut().push((peer, t)));
        Ok(())
    }
    fn hole_token(mark: Mark) -> PieResult<PieToken> {
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
                query: Some((TaskId(0), TaskId(1), mark)),
            })
        });
        Ok(t)
    }
    pub fn narrow(_: PieToken, _: Permission) -> PieResult<()> {
        if fake::NARROW_FAIL.with(|x| x.replace(false)) {
            Err(PieError)
        } else {
            Ok(())
        }
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
    pub fn ship(t: PieToken, to: TaskId, _: Access, policy: Policy) -> Result<To, ()> {
        if fake::SHIP_FAIL.with(|x| x.replace(false)) {
            return Err(());
        }
        let seed = fake::NEXT.with(|x| {
            let n = *x.borrow();
            *x.borrow_mut() += 1;
            PieToken(n)
        });
        fake::SHIPS.with(|x| x.borrow_mut().push((t, to)));
        if policy.0 == 1 {
            if let Some(h) = fake::HANDSHAKES.with(|x| x.borrow_mut().pop_front()) {
                let bootstrap = h.bootstrap.clone().unwrap_or_else(|| {
                    let mut v = Vec::new();
                    v.extend_from_slice(&(h.host.0 as u64).to_le_bytes());
                    v.extend_from_slice(&h.transport.to_bytes());
                    v
                });
                fake::push_message(t, bootstrap, h.bootstrap_sender);
                fake::ACTIVE.with(|x| *x.borrow_mut() = Some((t, h, false)));
            }
        }
        Ok(To(seed))
    }
}
#[path = "../../src/hand/receiver.rs"]
mod production_receiver;
#[path = "../../src/hand/sender.rs"]
mod production_sender;
pub mod hand {
    pub use crate::production_receiver::{Receiver, RecvFail, SourceFail};
    pub use crate::production_sender::{SendFail, Sender};
}
#[path = "../../src/session/mod.rs"]
pub mod session;
#[path = "../../src/time.rs"]
pub mod time;

#[cfg(test)]
mod tests {
    use super::*;
    use session::{
        self, Berth, establish,
        establish::{DiscoveryFail, EstablishFail},
    };
    fn matching(t: usize) {
        fake::add(t, true, 9, 4)
    }
    fn ready_handshake(host: usize, transport: usize, link_mark: usize, ask_mark: usize) {
        fake::add_transfer(transport, true, true, 9, 9, link_mark);
        fake::handshake(fake::Handshake {
            host: TaskId(host),
            transport: PieToken(transport),
            bootstrap: None,
            bootstrap_sender: TaskId(9),
            ack: vec![0],
            ack_sender: TaskId(host),
            advance_ms: 0,
        });
        let _ = ask_mark;
    }

    #[test]
    fn each_open_mints_fresh_pair_without_directory_scans_and_aliases_share_cleanup() {
        fake::reset();
        ready_handshake(11, 50, 4, 5);
        ready_handshake(12, 51, 4, 5);
        let berth = Berth {
            link: Mark(4),
            ask: Mark(5),
        };
        let first = session::Session::open(TaskId(9), berth, Wait::AtMost(20)).unwrap();
        let first_alias = first.clone();
        let second = session::Session::open(TaskId(9), berth, Wait::AtMost(20)).unwrap();
        assert_eq!(first.host(), TaskId(11));
        assert_eq!(second.host(), TaskId(12));
        assert_ne!(unsafe { first.raw_talk() }, unsafe { second.raw_talk() });
        assert_eq!(fake::scans(), 0);
        drop(first);
        assert!(fake::released().is_empty());
        drop(first_alias);
        assert_eq!(fake::released(), vec![PieToken(102), PieToken(100)]);
        drop(second);
        assert_eq!(
            fake::released(),
            vec![PieToken(102), PieToken(100), PieToken(106), PieToken(104)]
        );
    }

    #[test]
    fn open_uses_one_budget_for_bootstrap_and_ack() {
        fake::reset();
        ready_handshake(11, 50, 4, 5);
        fake::ACTIVE.with(|_| {});
        fake::HANDSHAKES.with(|q| {
            let mut q = q.borrow_mut();
            let mut h = q.pop_front().unwrap();
            h.advance_ms = 3;
            q.push_back(h);
        });
        let session = session::Session::open(
            TaskId(9),
            Berth {
                link: Mark(4),
                ask: Mark(5),
            },
            Wait::AtMost(10),
        )
        .unwrap();
        let waits = fake::WAITS.with(|x| x.borrow().clone());
        assert_eq!(waits, [Wait::AtMost(10), Wait::AtMost(7)]);
        drop(session);
    }

    #[test]
    fn open_rejects_malformed_or_wrong_source_bootstrap_and_releases_local_reply() {
        for (bytes, source) in [(Some(vec![0; 15]), TaskId(9)), (None, TaskId(8))] {
            fake::reset();
            ready_handshake(11, 50, 4, 5);
            fake::HANDSHAKES.with(|q| {
                let mut q = q.borrow_mut();
                let mut h = q.pop_front().unwrap();
                h.bootstrap = bytes;
                h.bootstrap_sender = source;
                q.push_back(h);
            });
            assert!(
                session::Session::open(
                    TaskId(9),
                    Berth {
                        link: Mark(4),
                        ask: Mark(5)
                    },
                    Wait::POLL
                )
                .is_err()
            );
            assert_eq!(fake::released(), vec![PieToken(100)]);
        }
    }

    #[test]
    fn open_rejects_bad_ack_source_or_code_and_releases_both_local_capabilities() {
        for (ack, source) in [
            (vec![1], TaskId(11)),
            (vec![0], TaskId(9)),
            (vec![0, 0], TaskId(11)),
        ] {
            fake::reset();
            ready_handshake(11, 50, 4, 5);
            fake::HANDSHAKES.with(|q| {
                let mut q = q.borrow_mut();
                let mut h = q.pop_front().unwrap();
                h.ack = ack;
                h.ack_sender = source;
                q.push_back(h);
            });
            assert!(
                session::Session::open(
                    TaskId(9),
                    Berth {
                        link: Mark(4),
                        ask: Mark(5)
                    },
                    Wait::POLL
                )
                .is_err()
            );
            assert_eq!(fake::released(), vec![PieToken(102), PieToken(100)]);
        }
    }
    #[test]
    fn transport_import_rejects_wrong_native_facts_without_releasing_the_foreign_capability() {
        for invalid in 0..5 {
            fake::reset();
            ready_handshake(11, 50, 4, 5);
            fake::CANDIDATES.with(|candidates| {
                let mut candidates = candidates.borrow_mut();
                let transport = candidates
                    .iter_mut()
                    .find(|candidate| candidate.token == PieToken(50))
                    .unwrap();
                match invalid {
                    0 => transport.alive = false,
                    1 => transport.hole = false,
                    2 => transport.query.as_mut().unwrap().0 = TaskId(8),
                    3 => transport.query.as_mut().unwrap().1 = TaskId(8),
                    _ => transport.query.as_mut().unwrap().2 = Mark(99),
                }
            });
            assert!(
                session::Session::open(
                    TaskId(9),
                    Berth {
                        link: Mark(4),
                        ask: Mark(5)
                    },
                    Wait::POLL
                )
                .is_err()
            );
            assert_eq!(fake::released(), vec![PieToken(100)]);
            assert!(fake::clean().is_empty());
        }
    }

    #[test]
    fn give_at_narrow_failure_revokes_only_its_delivery_and_releases_its_local_hole() {
        fake::reset();
        matching(2);
        fake::NARROW_FAIL.with(|fail| *fail.borrow_mut() = true);
        assert_eq!(
            establish::give_at(TaskId(9), Mark(4)),
            Err(EstablishFail::NoSeed)
        );
        assert_eq!(fake::released(), vec![PieToken(100)]);
        assert_eq!(fake::clean(), vec![(TaskId(9), PieToken(101))]);
    }

    struct Byte;
    impl wire::Message for Byte {
        type In = u8;
        type Buf = [u8; 1];
        const EMPTY: Self::Buf = [0];
        fn store(&self, bytes: &mut [u8]) -> Option<usize> {
            *bytes.first_mut()? = 1;
            Some(1)
        }
        fn fetch(bytes: &[u8]) -> Option<u8> {
            (bytes.len() == 1).then(|| bytes[0])
        }
    }
    struct ByteCall;
    impl wire::Contract for ByteCall {
        type Request = Byte;
        type Response = Byte;
    }

    #[test]
    fn imported_session_aliases_share_closed_state_and_borrowed_ownership() {
        fake::reset();
        let link = establish::lend(TaskId(9), Mark(4)).unwrap();
        let (talk, _) = establish::give_at(TaskId(11), Mark(5)).unwrap();
        // These fresh endpoints have no other sender, receiver, or session state.
        let imported = unsafe { session::Session::from_raw(link, talk, TaskId(11)) }.unwrap();
        let alias = imported.clone();
        assert!(matches!(
            imported.call::<ByteCall>(Byte, Wait::POLL),
            Err(session::CallFail::Receive(_))
        ));
        let sent = fake::PUSHES.with(|p| p.borrow().len());
        assert!(matches!(
            alias.call::<ByteCall>(Byte, Wait::POLL),
            Err(session::CallFail::Closed)
        ));
        assert_eq!(fake::PUSHES.with(|p| p.borrow().len()), sent);
        drop(imported);
        drop(alias);
        assert!(fake::released().is_empty());
    }

    #[test]
    fn from_raw_does_not_take_ownership_of_borrowed_capabilities() {
        fake::reset();
        let link = establish::lend(TaskId(9), Mark(4)).unwrap();
        let (talk, _) = establish::give_at(TaskId(11), Mark(5)).unwrap();
        let borrowed = unsafe { session::Session::from_raw(link, talk, TaskId(11)) }.unwrap();
        let alias = borrowed.clone();
        drop(borrowed);
        drop(alias);
        assert!(fake::released().is_empty());
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

#[derive(Clone, Copy)]
pub enum UnsealArgs { Hole { mark: Mark } }
impl UnsealArgs { pub fn hole(mark: Mark) -> Self { Self::Hole { mark } } }
#[derive(Clone, Copy)]
pub enum ReleaseMode { Revoke, Keep }

pub struct PieInfo { pub vestor: TaskId, pub owner: TaskId, pub mark: Mark, pub alive: bool }
