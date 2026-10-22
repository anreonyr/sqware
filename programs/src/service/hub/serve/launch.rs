use env::TaskId;
use ipc::rpc::request::Pending;
use system_api::control::{Call, Fail, State};

use super::peers::Peers;

pub(super) enum Reply {
    Pending,
    Admitted,
    Abort { trusted: bool },
}

/// One reported instance being authenticated and started through Control.
pub(super) struct Launch {
    task: TaskId,
    deadline: u64,
    authenticated: bool,
    pending: Option<Pending<Call>>,
}

impl Launch {
    pub(super) fn new(task: TaskId, deadline: u64) -> Self {
        Self {
            task,
            deadline,
            authenticated: false,
            pending: None,
        }
    }

    pub(super) fn task(&self) -> TaskId {
        self.task
    }

    pub(super) fn deadline(&self) -> u64 {
        self.deadline
    }

    pub(super) fn poll(&mut self, peers: &Peers, now: u64) -> Reply {
        if self.pending.is_none() {
            return self.begin(peers, now);
        }
        let mut pending = self.pending.take().expect("checked pending launch");
        let said = match pending.poll() {
            Ok(Some(said)) => said,
            Ok(None) => {
                self.pending = Some(pending);
                return if now >= self.deadline {
                    Reply::Abort {
                        trusted: self.authenticated,
                    }
                } else {
                    Reply::Pending
                };
            }
            Err(_) => return self.retry(now),
        };
        let instance = peers.control.instance(self.task);
        if !self.authenticated {
            return match instance.inspect_reply(said) {
                Ok(said) if State::of_code(said.a) != Some(State::Dead) && said.team.get() != 0 => {
                    self.authenticated = true;
                    self.begin(peers, now)
                }
                Ok(_) => Reply::Abort { trusted: true },
                Err(Fail::Unknown) | Err(Fail::Denied) => Reply::Abort { trusted: false },
                Err(_) => self.retry(now),
            };
        }
        match instance.embark_reply(said) {
            Ok(()) => Reply::Admitted,
            Err(Fail::NotReady) => Reply::Abort { trusted: true },
            Err(Fail::Unknown) | Err(Fail::Denied) => Reply::Abort { trusted: false },
            Err(_) => self.retry(now),
        }
    }

    fn begin(&mut self, peers: &Peers, now: u64) -> Reply {
        if now >= self.deadline {
            return Reply::Abort {
                trusted: self.authenticated,
            };
        }
        let instance = peers.control.instance(self.task);
        let wait = super::execution::budget(now, self.deadline);
        let pending = if self.authenticated {
            instance.begin_embark(wait)
        } else {
            instance.begin_inspect(wait)
        };
        match pending {
            Ok(pending) => {
                self.pending = Some(pending);
                Reply::Pending
            }
            Err(Fail::Unknown) | Err(Fail::Denied) => Reply::Abort { trusted: false },
            Err(_) => self.retry(now),
        }
    }

    fn retry(&mut self, now: u64) -> Reply {
        if now >= self.deadline {
            Reply::Abort {
                trusted: self.authenticated,
            }
        } else {
            Reply::Pending
        }
    }
}
