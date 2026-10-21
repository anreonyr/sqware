use env::{PieToken, TaskId};

/// Caller and sensor provenance have been checked by the native door adapter.
pub struct ClaimRequest {
    pub from: TaskId,
    pub id: u64,
    pub requester: u32,
    pub sensor: PieToken,
    pub recovery: TaskId,
}
