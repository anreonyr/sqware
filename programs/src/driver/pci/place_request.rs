use env::TaskId;

/// Recipient is supplied by the authenticated native adapter, not the wire.
#[derive(Clone, Copy)]
pub struct PlaceRequest {
    pub serial: u64,
    pub requester: u32,
    pub recipient: TaskId,
    pub bar: u8,
}
