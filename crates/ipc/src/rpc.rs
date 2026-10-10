//! Typed one-request, one-reply exchanges over a service entry.

use core::marker::PhantomData;
use env::{MailFail, Mark, PieFail, PieToken, TaskId, Wait};
use resource::{
    port::{Reply, ReplyError, Sender as PortSender},
    raw,
};
pub use wire::Contract;
use wire::Message;

use crate::{
    hand::{SendFail, Sender as HandSender},
    time::Deadline,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Open(PieFail),
    Grant(PieFail),
    Encode,
    Decode,
    Send(MailFail),
    Receive(MailFail),
    Untrusted,
    WrongSource,
}

pub mod request {
    use super::*;

    /// Client endpoint bound to one typed request/response contract.
    pub struct Sender<C: Contract> {
        entry: PortSender,
        token: PieToken,
        back: Mark,
        _contract: PhantomData<fn() -> C>,
    }

    impl<C: Contract> Sender<C> {
        pub fn from_raw(entry: PieToken, back: Mark) -> Result<Self, Fail> {
            let token = entry;
            let entry = PortSender::import(entry).map_err(|error| Fail::Open(error.source))?;
            Ok(Self {
                entry,
                token,
                back,
                _contract: PhantomData,
            })
        }

        pub fn peer(&self) -> TaskId {
            self.entry.peer()
        }

        pub fn begin(
            &self,
            deadline: Deadline,
            build: impl FnOnce(PieToken) -> C::Request,
        ) -> Result<Pending<C>, Fail> {
            let mut reply =
                Reply::open(self.peer(), self.back).map_err(|error| Fail::Open(error.source))?;
            let remote = reply.grant().map_err(|error| Fail::Grant(error.source))?;
            let request = build(remote);
            let mut buffer = C::Request::EMPTY;
            let len = request.store(buffer.as_mut()).ok_or(Fail::Encode)?;
            if len > buffer.as_ref().len() {
                return Err(Fail::Encode);
            }
            Ok(Pending {
                entry: PortSender::import(self.token).map_err(|error| Fail::Open(error.source))?,
                buffer,
                len,
                reply: super::reply::Receiver::new(reply, deadline),
                deadline,
                sent: false,
                done: false,
            })
        }

        /// Send one typed request and return its isolated one-shot reply endpoint.
        pub fn send(
            &self,
            deadline: Deadline,
            build: impl FnOnce(PieToken) -> C::Request,
        ) -> Result<super::reply::Receiver<C::Response>, Fail> {
            let mut reply =
                Reply::open(self.peer(), self.back).map_err(|error| Fail::Open(error.source))?;
            let remote = reply.grant().map_err(|error| Fail::Grant(error.source))?;
            let request = build(remote);
            let mut buffer = C::Request::EMPTY;
            let len = request.store(buffer.as_mut()).ok_or(Fail::Encode)?;
            let bytes = buffer.as_ref().get(..len).ok_or(Fail::Encode)?;
            self.entry
                .push(bytes, deadline.remaining())
                .map_err(|error| Fail::Send(error.source))?;
            Ok(super::reply::Receiver::new(reply, deadline))
        }

        /// Send a request and consume its one-shot reply within the same budget.
        pub fn call(
            &self,
            deadline: Deadline,
            build: impl FnOnce(PieToken) -> C::Request,
        ) -> Result<<C::Response as Message>::In, Fail> {
            self.send(deadline, build)?.receive()
        }
    }

    pub struct Pending<C: Contract> {
        entry: PortSender,
        buffer: <C::Request as Message>::Buf,
        len: usize,
        reply: super::reply::Receiver<C::Response>,
        deadline: Deadline,
        sent: bool,
        done: bool,
    }
    impl<C: Contract> Pending<C> {
        pub fn poll(&mut self) -> Result<Option<<C::Response as Message>::In>, Fail> {
            if self.done {
                return Err(Fail::Untrusted);
            }
            if !self.sent {
                match self
                    .entry
                    .push(&self.buffer.as_ref()[..self.len], Wait::POLL)
                {
                    Ok(()) => self.sent = true,
                    Err(error)
                        if error.source.is_busy() && self.deadline.remaining() != Wait::POLL =>
                    {
                        return Ok(None);
                    }
                    Err(error) => return Err(Fail::Send(error.source)),
                }
            }
            let result = self.reply.poll()?;
            if result.is_some() {
                self.done = true;
            }
            Ok(result)
        }
    }

    /// Server endpoint bound to one typed request/response contract.
    pub struct Receiver<C: Contract> {
        entry: PieToken,
        back: Mark,
        route: fn(&<C::Request as Message>::In) -> PieToken,
        _contract: PhantomData<fn() -> C>,
    }

    impl<C: Contract> Receiver<C> {
        pub fn from_raw(
            entry: PieToken,
            back: Mark,
            route: fn(&<C::Request as Message>::In) -> PieToken,
        ) -> Self {
            Self {
                entry,
                back,
                route,
                _contract: PhantomData,
            }
        }

        /// Receive, decode, and validate the reply route embedded in one request.
        pub fn receive(
            &self,
            buffer: &mut [u8],
            within: Wait,
        ) -> Result<Incoming<C>, Rejected<<C::Request as Message>::In>> {
            let (len, from) = match raw::Hole::from_raw(self.entry).pull_with(
                buffer,
                within,
                env::Oversize::Discard,
            ) {
                Ok(env::PullOutcome::Received { len, sender }) => (len, sender),
                Ok(env::PullOutcome::Discarded { .. }) => {
                    return Err(Rejected {
                        fail: Fail::Decode,
                        incoming: None,
                    });
                }
                Err(error) => {
                    return Err(Rejected {
                        fail: Fail::Receive(error.source),
                        incoming: None,
                    });
                }
            };
            let bytes = match buffer.get(..len) {
                Some(bytes) => bytes,
                None => {
                    return Err(Rejected {
                        fail: Fail::Decode,
                        incoming: None,
                    });
                }
            };
            let request = match C::Request::fetch(bytes) {
                Some(request) => request,
                None => {
                    return Err(Rejected {
                        fail: Fail::Decode,
                        incoming: None,
                    });
                }
            };
            let token = (self.route)(&request);
            let reply = match super::reply::Sender::<C::Response>::from_raw(token, from, self.back)
            {
                Ok(reply) => reply,
                Err(fail) => {
                    return Err(Rejected {
                        fail,
                        incoming: Some((from, request)),
                    });
                }
            };
            Ok(Incoming {
                from,
                request,
                reply,
                _contract: PhantomData,
            })
        }
    }

    pub struct Incoming<C: Contract> {
        pub from: TaskId,
        pub request: <C::Request as Message>::In,
        pub reply: super::reply::Sender<C::Response>,
        _contract: PhantomData<fn() -> C>,
    }

    /// A rejected request. Decoded request data is retained only when its reply route is untrusted.
    #[derive(Debug)]
    pub struct Rejected<T> {
        pub fail: Fail,
        pub incoming: Option<(TaskId, T)>,
    }
}

pub mod reply {
    use super::*;

    /// Typed, one-shot server-side reply route.
    pub struct Sender<R: Message> {
        token: PieToken,
        active: bool,
        _response: PhantomData<fn() -> R>,
    }

    impl<R: Message> Sender<R> {
        /// Import and validate an untrusted reply token at the receive boundary.
        pub fn from_raw(token: PieToken, from: TaskId, mark: Mark) -> Result<Self, Fail> {
            match raw::reserve(token) {
                Ok((vestor, owner, actual))
                    if vestor == from && owner == from && actual == mark =>
                {
                    Ok(Self {
                        token,
                        active: true,
                        _response: PhantomData,
                    })
                }
                _ => Err(Fail::Untrusted),
            }
        }

        /// Whether this imported reply route still names a live hole.
        pub fn is_alive(&self) -> bool {
            raw::alive(self.token)
        }

        /// Encode and send exactly one response, then release the imported route.
        pub fn send(mut self, response: R) -> Result<(), Fail> {
            let result = HandSender::<R>::from_raw(self.token)
                .send_within(response, Wait::POLL)
                .map_err(map_send);
            self.close();
            result
        }

        fn close(&mut self) {
            if self.active {
                self.active = false;
                let _ = raw::release(self.token);
            }
        }
    }

    impl<R: Message> Drop for Sender<R> {
        fn drop(&mut self) {
            self.close();
        }
    }

    /// Client-side reply endpoint. Receiving consumes it and its remote grant.
    pub struct Receiver<R: Message> {
        reply: Reply,
        deadline: Deadline,
        _response: PhantomData<fn() -> R>,
    }

    impl<R: Message> Receiver<R> {
        pub(super) fn new(reply: Reply, deadline: Deadline) -> Self {
            Self {
                reply,
                deadline,
                _response: PhantomData,
            }
        }

        pub fn poll(&self) -> Result<Option<R::In>, Fail> {
            let mut buffer = R::EMPTY;
            let bytes = match self.reply.pull(buffer.as_mut(), Wait::POLL) {
                Ok(bytes) => bytes,
                Err(ReplyError::Mail(fail))
                    if fail.is_busy() && self.deadline.remaining() != Wait::POLL =>
                {
                    return Ok(None);
                }
                Err(ReplyError::Mail(fail)) => return Err(Fail::Receive(fail)),
                Err(ReplyError::WrongSource) => return Err(Fail::WrongSource),
            };
            R::fetch(bytes).map(Some).ok_or(Fail::Decode)
        }

        pub fn receive(self) -> Result<R::In, Fail> {
            let mut buffer = R::EMPTY;
            let bytes = self
                .reply
                .pull(buffer.as_mut(), self.deadline.remaining())
                .map_err(|error| match error {
                    ReplyError::WrongSource => Fail::WrongSource,
                    ReplyError::Mail(fail) => Fail::Receive(fail),
                })?;
            R::fetch(bytes).ok_or(Fail::Decode)
        }
    }
}

fn map_send(error: SendFail) -> Fail {
    match error {
        SendFail::Mail(error) => Fail::Send(error),
        SendFail::TooLong | SendFail::Unbound => Fail::Encode,
    }
}
