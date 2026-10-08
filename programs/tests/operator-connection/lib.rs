#![allow(dead_code)]
extern crate alloc;
extern crate self as env;
extern crate self as ipc;
extern crate self as resource;
pub use abi::{MailFail, Mark, PieToken, TaskId, Wait};
pub mod wire {
    pub use abi::wire::Field;
}
use std::{cell::RefCell, collections::BTreeMap};
#[derive(Default)]
struct Backend {
    now: u64,
    next: usize,
    facts: BTreeMap<usize, (TaskId, TaskId, Mark)>,
    inbox: BTreeMap<usize, Vec<(Vec<u8>, TaskId)>>,
    offers: Vec<usize>,
    accepted: Vec<usize>,
    released: Vec<usize>,
    revoked: Vec<usize>,
    receivers: BTreeMap<usize, usize>,
    guests: Vec<(TaskId, PieToken, PieToken)>,
    waits: Vec<Wait>,
    tip_busy: bool,
}
thread_local! { static BACKEND: RefCell<Backend> = RefCell::new(Backend { next: 100, ..Backend::default() }); }
fn token(value: usize) -> PieToken {
    PieToken::mint(value)
}
fn setup(reply: usize, caller: usize) {
    BACKEND.with(|b| {
        b.borrow_mut().facts.insert(
            reply,
            (
                TaskId::new(caller),
                TaskId::new(caller),
                system_api::operator::LINK_MARK,
            ),
        );
    });
}
pub mod chrono {
    pub fn clock() -> u64 {
        crate::BACKEND.with(|b| b.borrow().now)
    }
}
pub mod pie {
    pub fn revoke(_: crate::TaskId, seed: crate::PieToken) -> Result<(), ()> {
        crate::BACKEND.with(|b| b.borrow_mut().revoked.push(seed.get()));
        Ok(())
    }
}
pub mod support {
    pub mod timing {
        pub const BOOT_MS: usize = 5000;
    }
}
#[derive(Debug)]
pub struct Error {
    pub source: MailFail,
}
pub mod raw {
    use super::*;
    pub fn alive(t: PieToken) -> bool {
        BACKEND.with(|b| b.borrow().facts.contains_key(&t.get()))
    }
    pub fn reserve(t: PieToken) -> Result<(TaskId, TaskId, Mark), ()> {
        BACKEND.with(|b| b.borrow().facts.get(&t.get()).copied().ok_or(()))
    }
    pub struct Entry {
        pub token: PieToken,
    }
    pub fn pies() -> impl Iterator<Item = Entry> {
        BACKEND
            .with(|b| {
                b.borrow()
                    .facts
                    .keys()
                    .map(|v| Entry { token: token(*v) })
                    .collect::<Vec<_>>()
            })
            .into_iter()
    }
    pub struct Hole(PieToken);
    impl Hole {
        pub fn from_raw(t: PieToken) -> Self {
            Self(t)
        }
        pub fn push(&self, _: &[u8], wait: Wait) -> Result<(), Error> {
            BACKEND.with(|b| {
                let mut b = b.borrow_mut();
                b.waits.push(wait);
                b.offers.push(self.0.get());
            });
            Ok(())
        }
        pub fn pull(&self, bytes: &mut [u8], wait: Wait) -> Result<(usize, TaskId), Error> {
            BACKEND.with(|b| {
                let mut b = b.borrow_mut();
                b.waits.push(wait);
                let queue = b.inbox.entry(self.0.get()).or_default();
                if queue.is_empty() {
                    return Err(Error {
                        source: MailFail::Busy,
                    });
                }
                let (frame, from) = queue.remove(0);
                if frame.len() > bytes.len() {
                    return Err(Error {
                        source: MailFail::Denied,
                    });
                }
                bytes[..frame.len()].copy_from_slice(&frame);
                Ok((frame.len(), from))
            })
        }
    }
}
pub mod port {
    pub use abi::{Access, Policy};
    pub struct To(crate::PieToken);
    impl To {
        pub fn seed(self) -> crate::PieToken {
            self.0
        }
    }
    pub fn ship(_: crate::PieToken, _: crate::TaskId, _: Access, _: Policy) -> Result<To, ()> {
        crate::BACKEND.with(|b| {
            let mut b = b.borrow_mut();
            b.next += 1;
            Ok(To(crate::token(b.next)))
        })
    }
}
pub mod time {
    pub use crate::production_time::*;
}
#[path = "../../../crates/ipc/src/time.rs"]
mod production_time;
pub mod session {
    pub mod establish {
        pub struct Endpoint {
            rx: crate::PieToken,
            seed: crate::PieToken,
        }
        impl Endpoint {
            pub fn rx(&self) -> crate::PieToken {
                self.rx
            }
            pub fn seed(&self) -> crate::PieToken {
                self.seed
            }
        }
        pub struct Held(pub Endpoint);
        impl core::ops::Deref for Held {
            type Target = Endpoint;
            fn deref(&self) -> &Endpoint {
                &self.0
            }
        }
        impl Drop for Held {
            fn drop(&mut self) {
                crate::BACKEND.with(|b| b.borrow_mut().released.push(self.rx().get()));
            }
        }
        pub fn accept(reply: crate::PieToken) -> Result<Endpoint, ()> {
            crate::BACKEND.with(|b| {
                let mut b = b.borrow_mut();
                b.next += 2;
                let rx = b.next;
                b.accepted.push(reply.get());
                b.receivers.insert(reply.get(), rx);
                Ok(Endpoint {
                    rx: crate::token(rx),
                    seed: crate::token(rx + 1),
                })
            })
        }
    }
}
pub mod hand {
    pub enum SendFail {
        Mail(crate::MailFail),
    }
    pub struct Sender<T>(core::marker::PhantomData<T>);
    impl Sender<system_api::operator::Tip> {
        pub fn from_raw(_: crate::PieToken) -> Self {
            Self(core::marker::PhantomData)
        }
        pub fn send_within(
            &self,
            tip: system_api::operator::Tip,
            wait: crate::Wait,
        ) -> Result<(), SendFail> {
            crate::BACKEND.with(|b| {
                let mut b = b.borrow_mut();
                b.waits.push(wait);
                if b.tip_busy {
                    return Err(SendFail::Mail(crate::MailFail::Busy));
                }
                if let system_api::operator::Tip::Guest { who, reply, ask } = tip {
                    b.guests.push((who, reply, ask));
                }
                Ok(())
            })
        }
    }
}
#[path = "../../src/system/operator/connection.rs"]
mod connection;
#[cfg(test)]
mod tests {
    use super::*;
    fn reset() {
        BACKEND.with(|b| {
            *b.borrow_mut() = Backend {
                next: 100,
                ..Backend::default()
            }
        });
    }
    fn address() -> (TaskId, PieToken) {
        (TaskId::new(10), token(50))
    }
    fn submit(reply: usize, caller: usize, bytes: Vec<u8>) {
        BACKEND.with(|b| {
            let mut b = b.borrow_mut();
            let rx = b.receivers[&reply];
            b.inbox
                .entry(rx)
                .or_default()
                .push((bytes, TaskId::new(caller)));
        });
    }
    #[test]
    fn stalled_peer_does_not_block_healthy_peer_or_restart_after_expiry() {
        reset();
        setup(2, 9);
        setup(3, 8);
        let mut book = connection::Connections::default();
        book.request(TaskId::new(9)).unwrap();
        book.request(TaskId::new(8)).unwrap();
        book.maintain(address()).unwrap();
        submit(3, 8, 91u64.to_le_bytes().to_vec());
        book.maintain(address()).unwrap();
        BACKEND.with(|b| {
            let b = b.borrow();
            assert_eq!(b.guests.len(), 1);
            assert_eq!(b.guests[0].0, TaskId::new(8));
            assert!(b.waits.iter().all(|w| *w == Wait::POLL));
        });
        assert_eq!(book.entries().count(), 1);
        BACKEND.with(|b| b.borrow_mut().now = 5_000_000_000);
        book.maintain(address()).unwrap();
        assert_eq!(book.entries().count(), 0);
        for _ in 0..3 {
            book.request(TaskId::new(9)).unwrap();
            book.maintain(address()).unwrap();
        }
        BACKEND.with(|b| {
            let b = b.borrow();
            assert_eq!(b.accepted, vec![2, 3]);
            assert_eq!(b.offers, vec![2, 3]);
            assert_eq!(b.released.len(), 1);
        });
    }
    #[test]
    fn malformed_request_is_terminal_and_cleans_only_local_transport() {
        reset();
        setup(2, 9);
        let mut book = connection::Connections::default();
        book.request(TaskId::new(9)).unwrap();
        book.maintain(address()).unwrap();
        submit(2, 8, 91u64.to_le_bytes().to_vec());
        book.maintain(address()).unwrap();
        book.request(TaskId::new(9)).unwrap();
        book.maintain(address()).unwrap();
        BACKEND.with(|b| {
            let b = b.borrow();
            assert_eq!(b.accepted, vec![2]);
            assert_eq!(b.released.len(), 1);
            assert!(b.guests.is_empty());
            assert!(b.revoked.is_empty());
        });
    }
    #[test]
    fn busy_handoff_expires_and_revokes_its_delivery_once() {
        reset();
        setup(2, 9);
        let mut book = connection::Connections::default();
        book.request(TaskId::new(9)).unwrap();
        book.maintain(address()).unwrap();
        submit(2, 9, 91u64.to_le_bytes().to_vec());
        BACKEND.with(|b| b.borrow_mut().tip_busy = true);
        book.maintain(address()).unwrap();
        assert_eq!(book.entries().count(), 1);
        BACKEND.with(|b| b.borrow_mut().now = 5_000_000_000);
        book.maintain(address()).unwrap();
        book.maintain(address()).unwrap();
        BACKEND.with(|b| {
            let b = b.borrow();
            assert_eq!(b.revoked.len(), 1);
            assert_eq!(b.released.len(), 1);
            assert!(b.guests.is_empty());
        });
    }
}
