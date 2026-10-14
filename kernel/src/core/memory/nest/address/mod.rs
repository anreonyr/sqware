//! 地址选择与占用：窗口只是选址范围，占用是全局单一事实。

mod occupancy;
mod placement;
mod placement_at;
mod region_id;
mod slot;
mod window;
pub(crate) use occupancy::Occupancy;
pub(crate) use placement::Placement;
pub(crate) use placement_at::PlacementAt;
pub(crate) use region_id::RegionId;
