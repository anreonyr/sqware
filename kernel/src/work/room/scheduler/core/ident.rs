use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{TaskId, TeamId};

use crate::memory::manager::addr::PhysAddr;
use crate::work::unit::task::TaskIdent;

use super::hart::frame_pa;
use super::table::{SCHEDULERS, current};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LastIdent {
    pub(crate) task: TaskId,
    pub(crate) team: TeamId,
}

impl LastIdent {
    const TAG: usize = 1;
    const TEAM_BITS: u32 = 31;
    const TASK_SHIFT: u32 = 1 + Self::TEAM_BITS;
    const TEAM_MASK: usize = (1 << Self::TEAM_BITS) - 1;

    const fn tagged(raw: usize) -> bool {
        raw & Self::TAG != 0
    }

    const fn pack(self) -> usize {
        Self::TAG | (self.team.get() << 1) | (self.task.get() << Self::TASK_SHIFT)
    }

    const fn unpack(raw: usize) -> LastIdent {
        LastIdent {
            task: TaskId::new(raw >> Self::TASK_SHIFT),
            team: TeamId::new((raw >> 1) & Self::TEAM_MASK),
        }
    }
}

#[derive(Clone, Copy)]
enum Payload {
    Empty,
    Live(*const TaskIdent),
    Last(LastIdent),
}

impl Payload {
    fn of(raw: usize) -> Payload {
        if raw == 0 {
            Payload::Empty
        } else if LastIdent::tagged(raw) {
            Payload::Last(LastIdent::unpack(raw))
        } else {
            Payload::Live(raw as *const TaskIdent)
        }
    }
}

pub enum Identity {
    Live(Arc<TaskIdent>),
    Last(LastIdent),
}

pub(super) struct Badge {
    raw: AtomicUsize,
}

impl Badge {
    pub(super) fn new() -> Badge {
        Badge {
            raw: AtomicUsize::new(0),
        }
    }

    pub(super) fn seat(&self, ident: &Arc<TaskIdent>) {
        let prev = self
            .raw
            .swap(Arc::into_raw(ident.clone()) as usize, Ordering::AcqRel);
        Self::reclaim(Payload::of(prev));
    }

    pub(super) fn shed(&self, ident: &TaskIdent) {
        let last = LastIdent {
            task: ident.id,
            team: ident.team.id,
        };
        let word = last.pack();
        debug_assert_eq!(LastIdent::unpack(word), last, "末次编码往返");
        let prev = Payload::of(self.raw.swap(word, Ordering::AcqRel));
        debug_assert!(matches!(prev, Payload::Live(_)), "shed 旧载荷非 Live");
        Self::reclaim(prev);
    }

    fn read(&self) -> Option<Identity> {
        match Payload::of(self.raw.load(Ordering::Acquire)) {
            Payload::Empty => None,
            Payload::Last(l) => Some(Identity::Last(l)),
            Payload::Live(p) => {
                // SAFETY: 未标签 = TaskIdent 载荷；槽持有者对 p 保有一份计数
                unsafe {
                    Arc::increment_strong_count(p);
                    Some(Identity::Live(Arc::from_raw(p)))
                }
            }
        }
    }

    fn reclaim(prev: Payload) {
        if let Payload::Live(p) = prev {
            // SAFETY: swap 取走后槽对其不再持有，此处 from_raw 收回该份计数
            unsafe {
                drop(Arc::from_raw(p));
            }
        }
    }
}

impl Identity {
    pub fn task_id(&self) -> usize {
        match self {
            Identity::Live(t) => t.id.get(),
            Identity::Last(l) => l.task.get(),
        }
    }

    pub fn team_id(&self) -> usize {
        match self {
            Identity::Live(t) => t.team.id.get(),
            Identity::Last(l) => l.team.get(),
        }
    }

    pub fn trap(&self) -> Option<PhysAddr> {
        match self {
            Identity::Live(t) => Some(frame_pa(t)),
            Identity::Last(_) => None,
        }
    }

    pub fn live(&self) -> Option<&TaskIdent> {
        match self {
            Identity::Live(t) => Some(t),
            Identity::Last(_) => None,
        }
    }
}

pub fn ident() -> Option<Identity> {
    SCHEDULERS.get()?;
    current().badge.read()
}