use env::{
    AcquireSource, FlatReference, MemoryResult, PieToken, Placement, ReferenceRange, TeamId,
    ViewControl, ViewId, ViewSelection,
};

use crate::mechanism::{mapping_kernel::MappingKernel, reference_kernel::ReferenceKernel};

/// 本套件唯一的引用与映射内核适配：全部落回同一份夹具状态。
pub struct FixtureKernel;

impl ReferenceKernel for FixtureKernel {
    fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<FlatReference> {
        crate::fixture_state::acquire(source, retention)
    }

    fn release(reference: FlatReference) -> MemoryResult<()> {
        crate::fixture_state::release(reference)
    }
}

impl MappingKernel for FixtureKernel {
    fn attach(
        range: ReferenceRange,
        team: TeamId,
        placement: Placement,
        access: u64,
        control: ViewControl,
    ) -> MemoryResult<(ViewId, usize)> {
        crate::fixture_state::attach(range, team, placement, access, control)
    }

    fn detach(team: TeamId, view: ViewId, selection: ViewSelection) -> MemoryResult<()> {
        crate::fixture_state::detach(team, view, selection)
    }

    fn set_access(
        team: TeamId,
        view: ViewId,
        selection: ViewSelection,
        access: u64,
    ) -> MemoryResult<()> {
        crate::fixture_state::set_access(team, view, selection, access)
    }
}
