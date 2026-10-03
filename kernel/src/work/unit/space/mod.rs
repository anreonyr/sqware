mod access;
mod backing;
mod inner;
mod map;
mod outer;
mod salvage;
mod segment;
pub(crate) mod window;

pub(crate) use backing::Backing;
pub(crate) use backing::Permit;
pub(crate) use map::{Pending, PendingState};
pub use outer::{Space, SpaceBuilder};
#[cfg(debug_assertions)]
pub(crate) use salvage::Salvage;
pub(crate) use salvage::Span;
pub(crate) use segment::SegmentKind;

pub(crate) fn inner_frame()
-> Result<crate::memory::manager::table::Frame, crate::memory::manager::MapError> {
    inner::SpaceInner::frame()
}

pub(crate) fn sync_instructions() -> Result<(), crate::memory::manager::MapError> {
    // SAFETY: callers have finished writing the pages before publishing executable mappings.
    unsafe { core::arch::asm!("fence rw, rw", options(nostack)) };
    crate::hart::request_instruction_sync();
    // SAFETY: order pending stores before reading leases; restore orders its lease before checking pending.
    unsafe { core::arch::asm!("fence rw, rw", "fence.i", options(nostack)) };
    let me = crate::hart::hart_id();
    let mut mask = 0usize;
    for h in (0..crate::hart::hart_count()).map(crate::hart::HartId::new) {
        if h != me && crate::memory::manager::asid::lease(h).is_some() {
            mask |= h.bit().1;
        }
    }
    sbi::RfenceCall::new(sbi::fid::Rfence::RemoteFenceI)
        .args(sbi::ecall::SArgs {
            a0: mask,
            a1: 0,
            ..Default::default()
        })
        .call()
        .map(|_| ())
        .map_err(|_| crate::memory::manager::MapError::NoRegion)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    Supervisor,
    User,
}

impl SpaceKind {
    pub fn is_supervisor(self) -> bool {
        matches!(self, SpaceKind::Supervisor)
    }
}

impl From<env::ProgramKind> for SpaceKind {
    fn from(k: env::ProgramKind) -> Self {
        match k {
            env::ProgramKind::User => SpaceKind::User,
            env::ProgramKind::Supervisor => SpaceKind::Supervisor,
        }
    }
}
