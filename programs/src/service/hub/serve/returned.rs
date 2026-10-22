use env::{TaskId, Wait};
use ipc::rpc::request::Pending;
use system_api::control::{Call, Fail};

use super::peers::Peers;
use super::return_reply::ReturnReply;

/// One owner receipt being returned to Control through the real instance face.
///
/// A launch that Hub never saw has no admission. It is authenticated once through
/// `StateInstance`; only a successful owner check creates a release request. Once
/// trusted, every reply failure keeps the same task and the same return obligation.
pub(super) struct Return {
    task: TaskId,
    deadline: u64,
    trusted: bool,
    pending: Option<Pending<Call>>,
}

impl Return {
    pub(super) fn new(task: TaskId, trusted: bool, deadline: u64) -> Self {
        Self {
            task,
            deadline,
            trusted,
            pending: None,
        }
    }

    pub(super) fn task(&self) -> TaskId {
        self.task
    }

    pub(super) fn trust(&mut self) {
        if self.trusted {
            return;
        }
        self.trusted = true;
        self.pending = None;
    }

    pub(super) fn poll(&mut self, peers: &Peers, now: u64) -> ReturnReply {
        if self.pending.is_none() {
            return self.begin(peers, now);
        }
        let mut pending = self.pending.take().expect("checked pending return");
        let said = match pending.poll() {
            Ok(Some(said)) => said,
            Ok(None) => {
                self.pending = Some(pending);
                return self.hold(now);
            }
            Err(_) => return self.begin(peers, now),
        };
        let instance = peers.control.instance(self.task);
        if !self.trusted {
            return match instance.inspect_reply(said) {
                Ok(_) => {
                    self.trusted = true;
                    self.begin(peers, now)
                }
                Err(Fail::Unknown) | Err(Fail::Denied) => ReturnReply::NoReceipt,
                Err(_) => self.hold(now),
            };
        }
        match instance.release_reply(said) {
            Ok(()) => ReturnReply::Released,
            // Any error reply keeps the same trusted obligation; never release it here.
            Err(_) => self.begin(peers, now),
        }
    }

    fn begin(&mut self, peers: &Peers, now: u64) -> ReturnReply {
        if !self.trusted && now >= self.deadline {
            return ReturnReply::NoReceipt;
        }
        let instance = peers.control.instance(self.task);
        let pending = if self.trusted {
            instance.begin_release(Wait::AtMost(super::execution::RETURN_MS))
        } else {
            instance.begin_inspect(super::execution::budget(now, self.deadline))
        };
        match pending {
            Ok(pending) => {
                self.pending = Some(pending);
                ReturnReply::Pending
            }
            Err(Fail::Unknown) | Err(Fail::Denied) if !self.trusted => ReturnReply::NoReceipt,
            Err(_) => self.hold(now),
        }
    }

    fn hold(&self, now: u64) -> ReturnReply {
        if !self.trusted && now >= self.deadline {
            ReturnReply::NoReceipt
        } else {
            ReturnReply::Pending
        }
    }
}
