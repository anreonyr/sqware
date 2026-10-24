use env::{PieToken, TaskId};
use hub_api::DeviceId;

/// Caller and sensor provenance have been checked by the native door adapter.
pub struct ClaimRequest {
    pub from: TaskId,
    pub id: u64,
    pub device: DeviceId,
    pub sensor: PieToken,
}
