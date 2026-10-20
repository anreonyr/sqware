pub use alloc::sync::{Arc, Weak};
use ::core::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
pub use std::time::Duration;

mod blocked;
mod event;
mod ident;
mod site;
mod sites;
mod task;

pub use blocked::Blocked;
pub use event::RoomEvent;
pub use site::Site;
pub use sites::Sites;
pub use task::Task;
pub use std::ops::Add;

pub type WakeKey = u64;
pub type Ticket = u64;
pub type WaitCall = ();
pub type Life = ();
pub type EventKind = RoomEvent;

thread_local! {
    pub static DOOM: Cell<bool> = const { Cell::new(false) };
    pub static INJECT: Cell<bool> = const { Cell::new(false) };
    pub static BEACON: Cell<bool> = const { Cell::new(false) };
    pub static PRESENT: Cell<bool> = const { Cell::new(true) };
    pub static OPS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}
static COMMIT: Mutex<()> = Mutex::new(());

pub fn record(op: &'static str) {
    OPS.with(|ops| ops.borrow_mut().push(op));
}

pub fn commit() -> MutexGuard<'static, ()> {
    if INJECT.with(|flag| flag.replace(false)) {
        DOOM.set(true);
        record("slay");
    }
    let guard = COMMIT.lock().unwrap();
    record("commit");
    guard
}

pub fn take_doomed(_: env::TaskId) -> Option<usize> {
    record("check");
    DOOM.get().then_some(123)
}

pub fn take_beacon(_: WakeKey) -> bool { BEACON.get() }
pub fn current() -> Task { Task::new() }
pub fn sites(_: WakeKey) -> &'static Sites { &Sites }
pub fn prune(_: &mut BTreeMap<WakeKey, Site>, _: WakeKey) {}

pub fn hold(_: Ticket, _: WakeKey, _: &Arc<Task>) -> Result<(), ()> {
    record("hold");
    Ok(())
}
pub fn void(_: Ticket) { record("void"); }
pub fn rise(_: impl Iterator<Item = Arc<Task>>) { record("rise"); }

pub trait WaitFail {
    fn busy() -> Self;
    fn oom() -> Self;
}
impl WaitFail for env::RoomFail {
    fn busy() -> Self { Self::Busy }
    fn oom() -> Self { Self::OoM }
}

pub trait Key {
    fn fold(self) -> u64;
    fn next_id() -> Self;
    fn raw(self) -> u64;
}
impl Key for u64 {
    fn fold(self) -> u64 { self }
    fn next_id() -> Self { 1 }
    fn raw(self) -> u64 { self }
}

pub trait Lifetime: Sized {
    fn dead(_: &Weak<Self>) -> bool;
}
impl Lifetime for () {
    fn dead(_: &Weak<Self>) -> bool { false }
}

pub trait Ticks {
    fn as_ticks(self) -> u64;
}
impl Ticks for Duration {
    fn as_ticks(self) -> u64 { self.as_nanos() as u64 }
}
pub mod clock {
    pub fn now() -> std::time::Duration { std::time::Duration::ZERO }
}
pub mod deadline {
    pub fn tock(_: u64, _: u64) -> Result<(), ()> {
        super::record("alarm");
        Ok(())
    }
}
pub mod trace {
    pub fn note(_: super::EventKind) { super::record("trace"); }
}

pub fn reset() {
    DOOM.set(false);
    INJECT.set(false);
    BEACON.set(false);
    PRESENT.set(true);
    OPS.with(|ops| ops.borrow_mut().clear());
    sites(1).lock().clear();
}
