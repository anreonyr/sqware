use hub_api::{BindingId, coordination::Call};
use ipc::rpc::request::Pending;

pub(super) struct BindingRpc {
    pub binding: BindingId,
    pub op: u8,
    pub pending: Pending<Call>,
}
