use env::{TaskId, Wait};
use ipc::rpc::request::Pending;
use system_api::control::{Call, Fail};

use super::peers::Peers;

pub(super) enum Reply {
    Pending,
    Released,
    NoReceipt,
}

/// One owner receipt being returned to Control through the real instance face.
///
/// A launch that Hub never saw has no admission. It is authenticated once through
/// `StateInstance`; only a successful owner check creates a release request.
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

    pub(super) fn poll(&mut self, peers: &Peers, now: u64) -> Reply {
        if self.pending.is_none() {
            return self.begin(peers, now);
        }
        let mut pending = self.pending.take().expect("checked pending return");
        let said = match pending.poll() {
            Ok(Some(said)) => said,
            Ok(None) => {
                self.pending = Some(pending);
                return if !self.trusted && now >= self.deadline {
                    Reply::NoReceipt
                } else {
                    Reply::Pending
                };
            }
            Err(_) => {
                return if !self.trusted && now >= self.deadline {
                    Reply::NoReceipt
                } else {
                    self.begin(peers, now)
                }
            }
        };
        let instance = peers.control.instance(self.task);
        if !self.trusted {
            return match instance.inspect_reply(said) {
                Ok(_) => {
                    self.trusted = true;
                    self.begin(peers, now)
                }
                Err(Fail::Unknown) | Err(Fail::Denied) => Reply::NoReceipt,
                Err(_) => {
                    if now >= self.deadline {
                        Reply::NoReceipt
                    } else {
                        Reply::Pending
                    }
                }
            };
        }
        match instance.release_reply(said) {
            Ok(()) => Reply::Released,
            Err(Fail::Unknown) | Err(Fail::Denied) => Reply::Released,
            Err(_) => self.begin(peers, now),
        }
    }

    fn begin(&mut self, peers: &Peers, now: u64) -> Reply {
        if !self.trusted && now >= self.deadline {
            return Reply::NoReceipt;
        }
        let instance = peers.control.instance(self.task);
        let pending = if self.trusted {
            let wait = Wait::AtMost(super::execution::RETURN_MS);
            instance.begin_release(wait)
        } else {
            instance.begin_inspect(super::execution::budget(now, self.deadline))
        };
        match pending {
            Ok(pending) => {
                self.pending = Some(pending);
                Reply::Pending
            }
            Err(Fail::Unknown) | Err(Fail::Denied) if !self.trusted => Reply::NoReceipt,
            Err(_) => {
                if !self.trusted && now >= self.deadline {
                    Reply::NoReceipt
                } else {
                    Reply::Pending
                }
            }
        }
    }
}
