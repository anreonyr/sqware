use alloc::sync::Arc;

use env::{TeamId, UnitFail};

use crate::core::work::commit;
use crate::core::work::unit::task::Task;

use super::TeamPhase;

pub(crate) fn oust(me: &Arc<Task>, id: TeamId) -> Result<(), UnitFail> {
    let child = me.clan(id).ok_or(UnitFail::Denied)?;
    child.retire().map_err(|_| UnitFail::Busy)?;
    let operation = child.operation().ok_or(UnitFail::Busy)?;
    {
        let _commit = commit();
        if !child.all_reaped() {
            return Err(UnitFail::Busy);
        }
    }
    let retired = {
        let _commit = commit();
        child.state.lock().phase = TeamPhase::Ousted;
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
