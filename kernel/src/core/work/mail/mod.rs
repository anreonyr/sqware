pub mod hole;
mod liveness;
mod nole_role;
pub mod nole;
pub mod pole;
pub mod tole;

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicUsize, Ordering};

use env::{Permission, PieFail, PieKind, PieToken, TaskId};

use crate::core::work::grant::Grant;
use crate::core::work::unit::life::Life;

pub(crate) use hole::HoleMeta;
pub(crate) use liveness::Liveness;
pub(crate) use nole::NoleMeta;
pub(crate) use pole::PoleMeta;
pub(crate) use tole::ToleMeta;

pub(crate) fn next_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub(crate) enum Mail {
    Hole(Arc<HoleMeta>),
    Pole(Arc<PoleMeta>),
    Nole(Arc<NoleMeta>),
    Tole(Arc<ToleMeta>),
}

impl Mail {
    pub(crate) fn kind(&self) -> PieKind {
        match self {
            Mail::Hole(_) => PieKind::Hole,
            Mail::Pole(_) => PieKind::Pole,
            Mail::Nole(_) => PieKind::Nole,
            Mail::Tole(_) => PieKind::Tole,
        }
    }

    pub(crate) fn owner(&self) -> TaskId {
        match self {
            Mail::Hole(meta) => meta.owner(),
            Mail::Pole(meta) => meta.owner(),
            Mail::Nole(meta) => meta.owner(),
            Mail::Tole(meta) => meta.owner(),
        }
    }

    pub(crate) fn id(&self) -> usize {
        match self {
            Mail::Hole(meta) => meta.id,
            Mail::Pole(meta) => meta.id,
            Mail::Nole(meta) => meta.id,
            Mail::Tole(meta) => meta.id,
        }
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        match self {
            Mail::Hole(meta) => meta.life(),
            Mail::Pole(meta) => meta.life(),
            Mail::Nole(meta) => meta.life(),
            Mail::Tole(meta) => meta.life(),
        }
    }

    pub(crate) fn alive(&self) -> bool {
        match self {
            Mail::Hole(meta) => meta.alive(),
            Mail::Pole(meta) => meta.alive(),
            Mail::Nole(meta) => meta.alive(),
            Mail::Tole(meta) => meta.alive(),
        }
    }

    pub(crate) fn seal(&self) {
        match self {
            Mail::Hole(meta) => hole::seal(meta),
            Mail::Pole(meta) => pole::seal(meta),
            Mail::Nole(meta) => nole::seal(meta),
            Mail::Tole(meta) => tole::seal(meta),
        }
    }

    pub(crate) fn hole(&self) -> Option<Arc<HoleMeta>> {
        match self {
            Mail::Hole(meta) => Some(meta.clone()),
            _ => None,
        }
    }

    pub(crate) fn pole(&self) -> Option<Arc<PoleMeta>> {
        match self {
            Mail::Pole(meta) => Some(meta.clone()),
            _ => None,
        }
    }

    pub(crate) fn nole(&self) -> Option<Arc<NoleMeta>> {
        match self {
            Mail::Nole(meta) => Some(meta.clone()),
            _ => None,
        }
    }

    pub(crate) fn tole(&self) -> Option<Arc<ToleMeta>> {
        match self {
            Mail::Tole(meta) => Some(meta.clone()),
            _ => None,
        }
    }

    pub(crate) fn same(&self, other: &Mail) -> bool {
        match (self, other) {
            (Mail::Hole(a), Mail::Hole(b)) => Arc::ptr_eq(a, b),
            (Mail::Pole(a), Mail::Pole(b)) => Arc::ptr_eq(a, b),
            (Mail::Nole(a), Mail::Nole(b)) => Arc::ptr_eq(a, b),
            (Mail::Tole(a), Mail::Tole(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub(crate) fn new_grant(
        &self,
        permission: Permission,
        token: PieToken,
        window: Option<(
            crate::core::memory::address::PageOffset,
            crate::core::memory::address::PageCount,
        )>,
    ) -> Result<Arc<Grant>, PieFail> {
        let grant = Grant::try_new_window(permission, window)?;
        if let Mail::Pole(meta) = self {
            meta.register_grant(token, grant.clone())?;
        }
        Ok(grant)
    }

    pub(crate) fn release_grant(&self, token: PieToken) {
        if let Mail::Pole(meta) = self {
            meta.release_grant(token);
        }
    }
}
