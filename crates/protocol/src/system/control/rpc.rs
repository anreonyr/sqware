/// Typed request and reply pairs for Control's public entry points.

use env::{Mark, PieToken};
use ipc::rpc::Contract as RpcContract;
use wire::Message;

/// Lifecycle requests sent to a Control service face.
pub struct Control;

impl RpcContract for Control {
    type Request = super::frame::Request;
    type Response = super::frame::Said;
    const BACK: Mark = super::frame::BACK;

    fn back(request: &<Self::Request as Message>::In) -> PieToken {
        request.1
    }
}

/// Publication requests sent to the publication face.
pub struct Publication;

impl RpcContract for Publication {
    type Request = super::publication::Frame;
    type Response = super::publication::Reply;
    const BACK: Mark = super::publication::BACK;

    fn back(request: &<Self::Request as Message>::In) -> PieToken {
        request.back
    }
}

/// Account requests sent to the account face.
pub struct Account;

impl RpcContract for Account {
    type Request = super::account::Request;
    type Response = crate::system::loader::frame::Said;
    const BACK: Mark = super::account::BACK;

    fn back(request: &<Self::Request as Message>::In) -> PieToken {
        request.0.back
    }
}
