use alloc::sync::Arc;

use env::{TeamId, UnitFail};

use crate::core::work::commit;
use crate::core::work::unit::task::Task;

use super::TeamPhase;

/// 撤离一座团队：先确认它已经没人活着、也没人正在里面做事，再撤下暂存表、标成
/// 已撤离、清掉家长位与收据，最后从家长的 heirs 里摘掉。
///
/// 次序在这里是判据：`operation` 租约活到 `life.clear` 之后；`life.prune` 之前先
/// 拿到 `sire` 引用（prune 会收窄家长的成员表）。
pub(crate) fn oust(me: &Arc<Task>, id: TeamId) -> Result<(), UnitFail> {
    let child = me.clan(id).ok_or(UnitFail::Denied)?;
    let operation = child.operation().ok_or(UnitFail::Busy)?;
    {
        let _commit = commit();
        if !child.all_reaped() {
            return Err(UnitFail::Busy);
        }
    }
    child.cancel_staging().map_err(|_| UnitFail::Denied)?;
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
