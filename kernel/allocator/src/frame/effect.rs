use crate::{InitError, MachineFacts};

/// Frame 算法需要的外部责任：机器事实、诊断、统计与分配禁令。
///
/// # Safety
///
/// 实现方返回的 `MachineFacts` 必须是其实际拥有的机器事实；帧池的 backing 在 Frame
/// 生存期内独占且有效，每次交付都匹配请求 `Layout` 的尺寸与对齐、且是尚未交付的
/// 新鲜区间；`bump_boundary`/`bump_frontier` 必须来自同一个实际 Bump 实例；该 effect
/// 只由一次性的私有装台构造，重复构造不得覆盖仍存活的 arena。
pub unsafe trait Effect: Sync {
    fn facts(&self) -> MachineFacts;
    fn bump_boundary(&self) -> usize;
    fn bump_frontier(&self) -> usize;
    fn install_frame_kinds(&self, frames: usize) -> Result<(), InitError>;
    fn record_frame_take(&self, index: usize, power: usize);
    fn record_frame_give(&self, index: usize);
    fn assert_allocation_allowed(&self);
    fn report(&self, args: core::fmt::Arguments<'_>);
}
