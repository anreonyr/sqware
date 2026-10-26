//! 真实 Env 适配：每个入口只转发一次，编码与算法仍在原 memory 函数。

use env::{
    AcquireSource, FlatReference, MemoryResult, PieToken, Placement, ReferenceRange, TeamId,
    ViewControl, ViewId, ViewSelection,
};

use super::mapping_kernel::MappingKernel;
use super::reference_kernel::ReferenceKernel;

/// 默认内核后端。
#[doc(hidden)]
pub struct Native;

impl ReferenceKernel for Native {
    fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<FlatReference> {
        super::acquire(source, retention)
    }

    fn release(reference: FlatReference) -> MemoryResult<()> {
        super::release(reference)
    }
}

impl MappingKernel for Native {
    fn attach(
        range: ReferenceRange,
        team: TeamId,
        placement: Placement,
        access: u64,
        control: ViewControl,
    ) -> MemoryResult<(ViewId, usize)> {
        super::attach(range, team, placement, access, control)
    }

    fn detach(team: TeamId, view: ViewId, selection: ViewSelection) -> MemoryResult<()> {
        super::detach(team, view, selection)
    }

    fn set_access(
        team: TeamId,
        view: ViewId,
        selection: ViewSelection,
        access: u64,
    ) -> MemoryResult<()> {
        super::set_access(team, view, selection, access)
    }
}
