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
    trusted: bool,
    target_received: bool,
    pending: Option<Pending<Call>>,
}

impl Return {
    pub(super) fn new(task: TaskId, trusted: bool, target_received: bool) -> Self {
        Self {
            task,
            trusted,
            target_received,
            pending: None,
        }
    }

    pub(super) fn task(&self) -> TaskId {
        self.task
    }

    pub(super) fn target(&mut self) {
        self.target_received = true;
    }

    pub(super) fn target_received(&self) -> bool {
        self.target_received
    }

    pub(super) fn trust(&mut self) {
        if self.trusted {
            return;
        }
        self.trusted = true;
        self.pending = None;
    }

    pub(super) fn poll(&mut self, peers: &Peers) -> ReturnReply {
        if self.pending.is_none() {
            return self.begin(peers);
        }
        let mut pending = self.pending.take().expect("checked pending return");
        let said = match pending.poll() {
            Ok(Some(said)) => said,
            Ok(None) => {
                self.pending = Some(pending);
                return ReturnReply::Pending;
            }
            Err(_) => return self.begin(peers),
        };
        let instance = peers.control.instance(self.task);
        if !self.trusted {
            return match instance.inspect_reply(said) {
                Ok(_) => {
                    self.trusted = true;
                    self.begin(peers)
                }
                Err(Fail::Unknown) | Err(Fail::Denied) => ReturnReply::NoReceipt,
                Err(_) => ReturnReply::Pending,
            };
        }
        match instance.release_reply(said) {
            Ok(()) => ReturnReply::Released,
            // Any error reply keeps the same trusted obligation; never release it here.
            Err(_) => self.begin(peers),
        }
    }

    fn begin(&mut self, peers: &Peers) -> ReturnReply {
        let instance = peers.control.instance(self.task);
        let pending = if self.trusted {
            instance.begin_release(Wait::AtMost(super::execution::RETURN_MS))
        } else {
            instance.begin_inspect(Wait::AtMost(super::execution::RETURN_MS))
        };
        match pending {
            Ok(pending) => {
                self.pending = Some(pending);
                ReturnReply::Pending
            }
            Err(Fail::Unknown) | Err(Fail::Denied) if !self.trusted => ReturnReply::NoReceipt,
            Err(_) => ReturnReply::Pending,
        }
    }
}
