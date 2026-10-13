mod access;
mod allocator;
mod space;

pub(crate) use allocator::RegionId;
#[cfg(debug_assertions)]
pub(crate) use space::fail_cleanup_prepare;
pub use space::{Mapping, Reservation, Space, SpaceBuilder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceKind {
    Supervisor,
    User,
}

impl SpaceKind {
    pub fn is_supervisor(self) -> bool {
        matches!(self, SpaceKind::Supervisor)
    }

    pub(crate) fn spp(self) -> riscv::register::sstatus::SPP {
        if self.is_supervisor() {
            riscv::register::sstatus::SPP::Supervisor
        } else {
            riscv::register::sstatus::SPP::User
        }
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
