mod access;
mod allocator;
mod space;

pub(crate) use allocator::RegionId;
#[cfg(debug_assertions)]
pub(crate) use space::fail_cleanup_prepare;
pub use space::{Mapping, Reservation, Space, SpaceBuilder};

pub(crate) fn sync_instructions() -> Result<(), crate::memory::manager::MapError> {
    unsafe { core::arch::asm!("fence rw, rw", options(nostack)) };
    crate::hart::request_instruction_sync();
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
