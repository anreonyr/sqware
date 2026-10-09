//! Exercise the production Connection with a service that revokes and replaces data grants.
#![allow(dead_code)]
extern crate alloc;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;
extern crate self as system_client;
pub use abi::{HoleDir, Mark, Permission, PieToken, TaskId, Wait};
pub mod wire {
    pub use abi::wire::*;
}
use abi::wire::Span;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};
use terminal_api::frame::{Command, Reply};
#[derive(Default)]
struct Native {
    next: usize,
    live: BTreeSet<usize>,
    replies: BTreeMap<usize, Vec<u8>>,
    loans: BTreeMap<usize, usize>,
    channels: [usize; 3],
}
thread_local! { static NATIVE: RefCell<Native> = RefCell::new(Native { next: 100, ..Native::default() }); }
fn token(n: usize) -> PieToken {
    PieToken::mint(n)
}
fn reset() {
    NATIVE.with(|n| {
        *n.borrow_mut() = Native {
            next: 100,
            ..Native::default()
        }
    });
}
pub mod debug {
    pub fn put(_: &str) {}
}
pub mod unit {
    pub fn self_id() -> super::TaskId {
        super::TaskId::new(32)
    }
}
pub mod pie {
    use super::*;
    pub fn unseal_hole(_: Mark) -> Result<PieToken, ()> {
        NATIVE.with(|n| {
            let mut n = n.borrow_mut();
            n.next += 1;
            Ok(token(n.next))
        })
    }
    pub fn accord(source: PieToken, _: TaskId, _: Permission, _: Mark) -> Result<PieToken, ()> {
        let remote = unseal_hole(Mark::NONE)?;
        NATIVE.with(|n| n.borrow_mut().loans.insert(remote.get(), source.get()));
        Ok(remote)
    }
    pub fn release(_: PieToken) -> Result<(), ()> {
        Ok(())
    }
    pub fn revoke(_: TaskId, _: PieToken) -> Result<(), ()> {
        Ok(())
    }
}
pub mod tole {
    pub fn unseal(_: bool) -> Result<super::PieToken, ()> {
        Ok(super::token(1))
    }
}
pub mod session {
    pub mod establish {
        pub fn find(
            _: super::super::TaskId,
            _: super::super::Mark,
        ) -> Result<super::super::PieToken, ()> {
            panic!("Connection must use the acknowledged seeds, not indexed discovery")
        }
    }
}
pub mod operator {
    pub struct Face;
    pub struct Tile;
    impl Face {
        pub fn tile(&self, _: &system_api::operator::Path, _: crate::Wait) -> Result<Tile, ()> {
            unreachable!()
        }
    }
    impl Tile {
        pub fn token(&self, _: crate::Wait) -> Result<crate::PieToken, ()> {
            unreachable!()
        }
    }
}
#[derive(Debug)]
pub struct Error {
    pub source: abi::MailFail,
}
fn denied() -> Error {
    Error {
        source: abi::MailFail::Denied,
    }
}
pub mod raw {
    use super::*;
    pub fn reserve(_: PieToken) -> Result<(TaskId, TaskId, Mark), ()> {
        unreachable!()
    }
    pub struct Hole(PieToken);
    impl Hole {
        pub fn from_raw(t: PieToken) -> Self {
            Self(t)
        }
        pub fn push(&self, bytes: &[u8], _: Wait) -> Result<(), Error> {
            NATIVE.with(|n| {
                let mut n = n.borrow_mut();
                if self.0 != token(5) {
                    return n
                        .live
                        .contains(&self.0.get())
                        .then_some(())
                        .ok_or_else(denied);
                }
                let (command, _) = Command::fetch_at(bytes, 0).unwrap();
                let rejected = command.task.get() == usize::MAX;
                let channels = if rejected {
                    [0; 3]
                } else {
                    let old = n.channels;
                    for t in old {
                        n.live.remove(&t);
                    }
                    if command.task == unit::self_id() {
                        let mut fresh = [0; 3];
                        for t in &mut fresh {
                            n.next += 1;
                            *t = n.next;
                            n.live.insert(*t);
                        }
                        n.channels = fresh;
                        fresh
                    } else {
                        n.channels = [0; 3];
                        [0; 3]
                    }
                };
                let reply = Reply {
                    status: u8::from(rejected),
                    authority: token(50),
                    input: token(channels[0]),
                    output: token(channels[1]),
                    control: token(channels[2]),
                };
                let mut bytes = vec![0; Reply::LEN];
                reply.store_at(&mut bytes, 0).unwrap();
                let back = n.loans[&command.back.get()];
                n.replies.insert(back, bytes);
                Ok(())
            })
        }
        pub fn pull(&self, bytes: &mut [u8], _: Wait) -> Result<(usize, TaskId), Error> {
            NATIVE.with(|n| {
                let reply = n
                    .borrow_mut()
                    .replies
                    .remove(&self.0.get())
                    .ok_or_else(denied)?;
                bytes[..reply.len()].copy_from_slice(&reply);
                Ok((reply.len(), TaskId::new(31)))
            })
        }
        pub fn wait(&self, _: HoleDir, _: Wait) -> Result<bool, Error> {
            Ok(true)
        }
    }
}
pub mod pile {
    use super::*;
    pub struct Pile;
    impl Pile {
        pub fn unseal(_: bool) -> Result<Self, ()> {
            Ok(Self)
        }
        pub fn attach(&self, t: PieToken, _: HoleDir) -> Result<(), ()> {
            NATIVE.with(|n| n.borrow().live.contains(&t.get()).then_some(()).ok_or(()))
        }
        pub fn await_(&self, _: Wait) -> Result<(), ()> {
            unreachable!()
        }
        pub fn token(&self) -> PieToken {
            super::token(1)
        }
    }
}
mod production {
    include!("../../src/user/terminal/client/src/terminal.rs");
    #[cfg(test)]
    mod tests {
        use super::*;
        fn open() -> Connection {
            crate::reset();
            Connection::open(Terminal {
                entry: crate::token(5),
                host: TaskId::new(31),
            })
            .unwrap()
        }
        #[test]
        fn attachment_uses_explicit_seeds_without_discovery() {
            let connection = open();
            connection.io().unwrap().write(b"attached").unwrap();
        }
        #[test]
        fn rejected_target_restores_fresh_seeds_and_revokes_old_io() {
            let mut connection = open();
            let old = connection.io().unwrap();
            assert!(connection.lend(TaskId::new(usize::MAX)).is_err());
            assert!(old.write(b"revoked").is_err());
            connection.io().unwrap().write(b"restored").unwrap();
        }
        #[test]
        fn foreground_restore_replaces_cached_seeds() {
            let mut connection = open();
            let old = connection.io().unwrap();
            let foreground = connection.lend(TaskId::new(9)).unwrap();
            assert!(old.write(b"revoked").is_err());
            foreground.restore().unwrap();
            connection.io().unwrap().write(b"restored").unwrap();
        }
    }
}
