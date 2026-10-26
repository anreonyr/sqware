//! 映射机制对内核的静态缝：与引用持有分开，只持有引用的夹具无须实现映射。

use env::{MemoryResult, Placement, ReferenceRange, TeamId, ViewControl, ViewId, ViewSelection};

/// 映射的单次内核调用。
pub trait MappingKernel {
    fn attach(
        range: ReferenceRange,
        team: TeamId,
        placement: Placement,
        access: u64,
        control: ViewControl,
    ) -> MemoryResult<(ViewId, usize)>;
    fn detach(team: TeamId, view: ViewId, selection: ViewSelection) -> MemoryResult<()>;
    fn set_access(
        team: TeamId,
        view: ViewId,
        selection: ViewSelection,
        access: u64,
    ) -> MemoryResult<()>;
}
