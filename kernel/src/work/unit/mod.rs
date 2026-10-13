static COMMIT: crate::lock::RelLock<()> =
    crate::lock::RelLock::new_level(crate::lock::Level::Unit, ());

#[cfg(debug_assertions)]
pub(crate) fn assert_unlocked() {
    assert!(
        !COMMIT.held_by_current(),
        "idle wait while holding Unit commit lock"
    );
}

pub(crate) fn commit() -> crate::lock::reentrant::RelLockGuard<'static, ()> {
    COMMIT.lock()
}

pub(crate) mod boot;
pub(crate) mod capsule;
pub(crate) mod copies;
pub(crate) mod join;
pub(crate) mod life;
pub mod space;
pub(crate) mod task;
pub(crate) mod team;

use crate::memory::manager::MapError;
use alloc::sync::Arc;

use space::SpaceBuilder;

pub type MapResult<T> = erra::Result<T, MapError>;

pub(crate) fn build(
    kind: space::SpaceKind,
    sire: task::TaskWeak,
) -> Result<Arc<team::Team>, MapError> {
    let space = match kind {
        space::SpaceKind::Supervisor => SpaceBuilder::supervisor(),
        space::SpaceKind::User => SpaceBuilder::user(),
    }
    .build()?;
    team::TeamBuilder::new(space)
        .sire(sire)
        .constructing()
        .spawn()
}

pub(crate) fn oust(me: &Arc<task::Task>, id: env::TeamId) -> Result<(), env::UnitFail> {
    let child = me.clan(id).ok_or(env::UnitFail::Denied)?;
    let operation = child.operation().ok_or(env::UnitFail::Busy)?;
    {
        let _commit = commit();
        if !child.all_reaped() {
            return Err(env::UnitFail::Busy);
        }
    }
    child.cancel_staging().map_err(|_| env::UnitFail::Denied)?;
    let retired = {
        let _commit = commit();
        child.state.lock().phase = team::TeamPhase::Ousted;
        child.life.clear(me.ident.id, None);
        let sire = child.sire.upgrade();
        let detached = sire.as_ref().and_then(|sire| sire.oust(id));
        child.life.prune();
        (sire, detached)
    };
    drop(operation);
    drop(retired);
    Ok(())
}
