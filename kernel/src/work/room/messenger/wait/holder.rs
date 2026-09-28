use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use hashbrown::HashMap;

use crate::lock::{Level, OnceLock, SpinLock};
use crate::runtime::chrono::timer;
use crate::work::unit::task::Task;
use crate::work::unit::weak::{Site, TaskWeak};

use super::site::WakeKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(pub(super) u64);

impl Ticket {
    pub(super) fn alloc() -> Ticket {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Ticket(NEXT.fetch_add(1, Ordering::Relaxed) as u64)
    }

    pub(super) fn raw(self) -> u64 {
        self.0
    }
}

type HolderTable = SpinLock<HashMap<Ticket, (WakeKey, TaskWeak)>>;

pub(in super::super) fn holders() -> &'static HolderTable {
    static HOLDERS: OnceLock<HolderTable> = OnceLock::new();
    HOLDERS.get_or_init(|| SpinLock::new_level(Level::L3, HashMap::new()))
}

pub(super) fn hold(ticket: Ticket, key: WakeKey, task: &Arc<Task>) -> Result<(), ()> {
    let mut table = holders().lock();
    table.try_reserve(1).map_err(|_| ())?;
    table.insert(
        ticket,
        (key, TaskWeak::stored(Arc::downgrade(task), Site::Holder)),
    );
    Ok(())
}

pub(in super::super) fn void(ticket: Ticket) -> Option<(WakeKey, Arc<Task>)> {
    timer::mute(ticket.raw());
    holders()
        .lock()
        .remove(&ticket)
        .and_then(|(key, w)| w.upgrade().map(|t| (key, t)))
}