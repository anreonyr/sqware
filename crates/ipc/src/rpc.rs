//! Single-request, single-reply exchanges over a service entry.

use env::{MailFail, Mark, PieFail, PieToken, TaskId, Wait, pie};
use resource::{port::{Reply, ReplyError, Sender}, raw};
use wire::Message;

use crate::{hand::{Sender as HandSender, SendFail}, time::Deadline};

/// An entry-backed client. Each call creates a new one-shot reply endpoint.
pub struct Client {
    entry: Sender,
    back: Mark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenFail {
    Entry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallFail {
    Open(PieFail),
    Grant(PieFail),
    Encode,
    Send(MailFail),
    Receive(MailFail),
    WrongSource,
    Malformed,
}

impl Client {
    /// Import an entry and bind reply-source checks to the entry's owner.
    pub fn from_raw(entry: PieToken, back: Mark) -> Result<Self, OpenFail> {
        let entry = Sender::import(entry).map_err(|_| OpenFail::Entry)?;
        Ok(Self { entry, back })
    }

    pub fn peer(&self) -> TaskId {
        self.entry.peer()
    }

    /// Encode and send one typed request, then receive a typed reply under the caller's budget.
    /// A failed exchange is never reused, so a late response cannot satisfy a later call.
    pub fn call<Q: Message, R: Message>(
        &self,
        deadline: &Deadline,
        build: impl FnOnce(PieToken) -> Q,
    ) -> Result<R::In, CallFail> {
        let reply = Reply::open(self.peer(), self.back).map_err(|error| CallFail::Open(error.source))?;
        let loan = reply.grant().map_err(|error| CallFail::Grant(error.source))?;
        let request = build(loan.remote());
        let mut request_buffer = Q::EMPTY;
        let len = request.store(request_buffer.as_mut()).ok_or(CallFail::Encode)?;
        let bytes = request_buffer.as_ref().get(..len).ok_or(CallFail::Encode)?;
        self.entry
            .push(bytes, deadline.remaining())
            .map_err(|error| CallFail::Send(error.source))?;

        let mut response = R::EMPTY;
        let bytes = reply
            .pull(response.as_mut(), deadline.remaining())
            .map_err(|error| match error {
                ReplyError::WrongSource => CallFail::WrongSource,
                ReplyError::Mail(fail) => CallFail::Receive(fail),
            })?;
        R::fetch(bytes).ok_or(CallFail::Malformed)
    }
}

/// One decoded request and the identity stamped by the kernel on its delivery.
pub struct Incoming<T> {
    pub from: TaskId,
    pub request: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiveFail {
    Mail(MailFail),
    Malformed(usize),
}

/// Receive and decode one request from a service's entry hole.
pub fn receive<M: Message>(
    entry: PieToken,
    buffer: &mut M::Buf,
    within: Wait,
) -> Result<Incoming<M::In>, ReceiveFail> {
    let (len, from) = raw::Hole::from_raw(entry)
        .pull(buffer.as_mut(), within)
        .map_err(|error| ReceiveFail::Mail(error.source))?;
    let bytes = buffer.as_ref().get(..len).ok_or(ReceiveFail::Malformed(len))?;
    let request = M::fetch(bytes).ok_or(ReceiveFail::Malformed(len))?;
    Ok(Incoming { from, request })
}

/// A source-checked reply route owned by a server workflow until it replies or drops it.
pub struct ReplyTo {
    token: PieToken,
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyFail {
    Untrusted,
    Encode,
    Send(MailFail),
}

impl ReplyTo {
    /// Validate a raw reply token before retaining it beyond the current request handler.
    /// Invalid tokens are deliberately left untouched because they may not belong to this request.
    pub fn from_raw(token: PieToken, from: TaskId, mark: Mark) -> Result<Self, ReplyFail> {
        match raw::reserve(token) {
            Ok((vestor, owner, actual)) if vestor == from && owner == from && actual == mark => {
                Ok(Self { token, active: true })
            }
            _ => Err(ReplyFail::Untrusted),
        }
    }

    /// Send exactly one response without waiting for the client to consume it.
    pub fn send<M: Message>(mut self, response: M) -> Result<(), ReplyFail> {
        let result = HandSender::<M>::from_raw(self.token)
            .send_within(response, Wait::POLL)
            .map_err(map_send);
        self.close();
        result
    }

    fn close(&mut self) {
        if self.active {
            self.active = false;
            let _ = pie::release(self.token);
        }
    }
}

impl Drop for ReplyTo {
    fn drop(&mut self) {
        self.close();
    }
}

fn map_send(error: SendFail) -> ReplyFail {
    match error {
        SendFail::Mail(error) => ReplyFail::Send(error),
        SendFail::TooLong | SendFail::Unbound => ReplyFail::Encode,
    }
}
