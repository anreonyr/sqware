use alloc::vec::Vec;

use super::Staging;
use super::Team;

/// 暂存表的租约：摘下团队的暂存表，退出（含失败）时原样还回去。
pub(crate) struct StagingLease<'a> {
    pub(crate) team: &'a Team,
    pub(crate) items: Vec<Staging>,
}

impl<'a> StagingLease<'a> {
    pub(crate) fn hold(team: &'a Team) -> StagingLease<'a> {
        StagingLease {
            team,
            items: team.take_staging(),
        }
    }
}

impl Drop for StagingLease<'_> {
    fn drop(&mut self) {
        self.team.restore_staging(core::mem::take(&mut self.items));
    }
}
