use alloc::sync::{Arc, Weak};
use core::time::Duration;

use env::{NoleFail, TaskId};

use super::Liveness;
use super::nole_role::NoleRole;
use crate::machine::sync::{Level, SpinLock};

use crate::core::work::room::messenger::{self, Handoff, WakeKey};
use crate::core::work::unit::life::Life;

pub struct NoleMeta {
    state: SpinLock<Liveness>,
    pub(crate) id: usize,
    life: Arc<Life>,
    ring: SpinLock<bool>,
    owner: TaskId,
    role: NoleRole,
}

impl NoleMeta {
    #[cfg(debug_assertions)]
    pub(crate) fn new(owner: TaskId) -> Arc<Self> {
        Self::try_new(owner).expect("nole allocation failed")
    }

    /// 内核铸的保留授权 Nole：只有经这扇门造出来的对象 `retains()` 为真。
    /// 这是内核的**真铸造厂**（不是调试期的假正面）；初始收货方由引导交付那一处指定，
    /// 普通请求调不到这一格。
    pub(crate) fn mint_retention(
        owner: TaskId,
    ) -> Result<Arc<Self>, crate::core::memory::nest::NestError> {
        Self::try_build(owner, NoleRole::Retention)
    }

    pub(crate) fn mint_guard(
        owner: TaskId,
    ) -> Result<Arc<Self>, crate::core::memory::nest::NestError> {
        Self::try_build(owner, NoleRole::Guard)
    }

    /// 是否为内核认可的保留授权对象。
    pub(crate) fn retains(&self) -> bool {
        matches!(self.role, NoleRole::Retention)
    }

    pub(crate) fn guards(&self) -> bool {
        matches!(self.role, NoleRole::Guard)
    }

    pub(crate) fn try_new(
        owner: TaskId,
    ) -> Result<Arc<Self>, crate::core::memory::nest::NestError> {
        Self::try_build(owner, NoleRole::Plain)
    }

    fn try_build(
        owner: TaskId,
        role: NoleRole,
    ) -> Result<Arc<Self>, crate::core::memory::nest::NestError> {
        let life = Life::try_new()?;
        Arc::try_new(Self {
            state: SpinLock::new_level(Level::L3, Liveness::Live),
            id: super::next_id(),
            life,
            ring: SpinLock::new_level(Level::L3, false),
            owner,
            role,
        })
        .map_err(|_| crate::core::memory::nest::NestError::OutOfMemory)
    }

    pub(crate) fn life(&self) -> Weak<Life> {
        Arc::downgrade(&self.life)
    }

    pub(crate) fn owner(&self) -> TaskId {
        self.owner
    }

    pub(crate) fn alive(&self) -> bool {
        matches!(*self.state.lock(), Liveness::Live)
    }

    pub(crate) fn ready(&self) -> bool {
        *self.ring.lock()
    }
}

impl Drop for NoleMeta {
    fn drop(&mut self) {
        seal(self);
    }
}

pub(crate) fn key(meta: &NoleMeta) -> WakeKey {
    WakeKey::Nole { id: meta.id }
}

pub(crate) fn ring(meta: &NoleMeta) -> Result<(), NoleFail> {
    if !meta.alive() {
        return Err(NoleFail::Dead);
    }
    {
        let mut ring = meta.ring.lock();
        if *ring {
            return Err(NoleFail::Busy);
        }
        *ring = true;
    }
    let _ = messenger::wake(key(meta), &meta.life());
    Ok(())
}

pub(crate) fn hush(meta: &NoleMeta) -> Result<(), NoleFail> {
    let mut ring = meta.ring.lock();
    if !*ring {
        return Err(NoleFail::Busy);
    }
    *ring = false;
    Ok(())
}

pub(crate) fn wait(meta: &NoleMeta, dur: Duration) -> Result<Handoff<bool>, NoleFail> {
    if !meta.alive() {
        return Err(NoleFail::Dead);
    }
    if meta.ready() {
        return Ok(Handoff::Resume(true));
    }
    if dur == Duration::ZERO {
        return Ok(Handoff::Resume(false));
    }
    Ok(match messenger::wait(key(meta), meta.life(), dur)? {
        Handoff::Resume(()) => Handoff::Resume(meta.alive() && meta.ready()),
        Handoff::Switch(pa) => Handoff::Switch(pa),
    })
}

pub(crate) fn seal(meta: &NoleMeta) {
    *meta.state.lock() = Liveness::Dead;
    messenger::wipe(WakeKey::seal(env::PieKind::Nole, meta.id));
    messenger::wipe(key(meta));
}
