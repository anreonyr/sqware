pub mod hole;
mod liveness;
pub mod nole;
pub mod pole;
pub mod tole;

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use env::{Bit, MailCondition, MailFail, Permission, PieFail, PieKind, PieToken, TaskId};

use crate::core::work::grant::Grant;
use crate::core::work::room::messenger::{Handoff, WakeKey};
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

    pub(crate) fn ring(&self, bit: Bit) -> Result<(), MailFail> {
        match (self, bit) {
            (Mail::Hole(meta), Bit::FIRST) => hole::ring(meta),
            (Mail::Nole(meta), Bit::FIRST) => nole::ring(meta),
            (Mail::Pole(meta), bit) => pole::ring(meta, bit),
            _ => Err(MailFail::Denied),
        }
    }

    pub(crate) fn hush(&self, bit: Bit) -> Result<(), MailFail> {
        match (self, bit) {
            (Mail::Hole(meta), Bit::FIRST) => hole::hush(meta),
            (Mail::Nole(meta), Bit::FIRST) => nole::hush(meta),
            (Mail::Pole(meta), bit) => pole::hush(meta, bit),
            _ => Err(MailFail::Denied),
        }
    }

    pub(crate) fn wait(
        &self,
        condition: MailCondition,
        dur: Duration,
    ) -> Result<Handoff<bool>, MailFail> {
        match (self, condition) {
            (
                Mail::Hole(meta),
                MailCondition::Pull | MailCondition::Push | MailCondition::Empty,
            ) => hole::wait(meta, condition, dur),
            (Mail::Nole(meta), MailCondition::Pull) => nole::wait(meta, dur),
            (Mail::Pole(meta), MailCondition::Signal(bit)) => pole::wait(meta, bit, dur),
            _ => Err(MailFail::Denied),
        }
    }

    pub(crate) fn ready(&self, condition: MailCondition) -> bool {
        match (self, condition) {
            (
                Mail::Hole(meta),
                MailCondition::Pull | MailCondition::Push | MailCondition::Empty,
            ) => meta.ready(condition),
            (Mail::Nole(meta), MailCondition::Pull) => meta.ready(),
            (Mail::Pole(meta), MailCondition::Signal(bit)) => meta.ready(bit),
            _ => false,
        }
    }

    pub(crate) fn key(&self, condition: MailCondition) -> Option<(WakeKey, Weak<Life>)> {
        match (self, condition) {
            (
                Mail::Hole(meta),
                MailCondition::Pull | MailCondition::Push | MailCondition::Empty,
            ) => Some((hole::key(meta, condition), meta.life())),
            (Mail::Nole(meta), MailCondition::Pull) => Some((nole::key(meta), meta.life())),
            (Mail::Pole(meta), MailCondition::Signal(bit)) => {
                Some((pole::key(meta, bit), meta.life()))
            }
            _ => None,
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
    ) -> Result<Arc<Grant>, PieFail> {
        let grant = Grant::try_new(permission)?;
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

pub(crate) fn mail_fail(error: env::PieFail) -> env::MailFail {
    match error {
        env::PieFail::Denied => env::MailFail::Denied,
        env::PieFail::Dead => env::MailFail::Dead,
        env::PieFail::OoM => env::MailFail::OoM,
        env::PieFail::NotAligned => env::MailFail::Denied,
        env::PieFail::HandedOver => env::MailFail::HandedOver,
        env::PieFail::Busy => env::MailFail::Busy,
    }
}
