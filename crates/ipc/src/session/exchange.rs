/// Typed request/response exchange over a session's existing endpoint.

use env::Wait;
use wire::Message;

use crate::hand::{Sender, SendFail, SourceFail};
use crate::time::Deadline;

use super::Session;

/// Message types which form one request/response exchange.
pub trait Contract {
    type Request: Message;
    type Response: Message;
}

/// The stage at which a session exchange failed.
#[derive(Debug)]
pub enum CallFail {
    Send(SendFail),
    Receive(SourceFail),
}

/// Shared implementation used by `Session::call` and host-side transport fixtures.
pub(crate) fn call<C: Contract>(
    session: &Session,
    request: C::Request,
    within: Wait,
) -> Result<<C::Response as Message>::In, CallFail> {
    let deadline = Deadline::new(within);
    let mut sender = Sender::<C::Request>::from_raw(session.talk);
    sender
        .send_within(request, deadline.remaining())
        .map_err(CallFail::Send)?;

    let receiver = session.link.receiver::<C::Response>();
    let mut buffer = <C::Response as Message>::EMPTY;
    receiver
        .recv_from(session.host, buffer.as_mut(), deadline.remaining())
        .map_err(CallFail::Receive)
}
