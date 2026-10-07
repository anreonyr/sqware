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
    Busy,
    Closed,
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
    let mut guard = super::state::begin(&session.state, session.link.rx(), session.talk)
        .map_err(|error| match error {
            super::state::GateFail::Busy => CallFail::Busy,
            super::state::GateFail::Closed => CallFail::Closed,
        })?;
    let mut sender = Sender::<C::Request>::from_raw(session.talk);
    sender
        .send_within(request, deadline.remaining())
        .map_err(CallFail::Send)?;
    guard.sent();

    let receiver = session.link.receiver::<C::Response>();
    let mut buffer = <C::Response as Message>::EMPTY;
    match receiver
        .recv_from(session.host, buffer.as_mut(), deadline.remaining())
    {
        Ok(response) => {
            guard.complete();
            Ok(response)
        }
        Err(error) => {
            guard.close();
            Err(CallFail::Receive(error))
        }
    }
}
