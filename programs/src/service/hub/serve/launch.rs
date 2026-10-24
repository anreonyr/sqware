use env::TaskId;
use ipc::rpc::request::Pending;
use system_api::control::{Call, Fail, State};

use crate::service::hub::core::Ledger;

use super::launch_phase::LaunchPhase;
use super::launch_reply::LaunchReply;
use super::peers::Peers;

/// One reported instance being authenticated and started through Control.
///
/// Owner authentication creates the bounded admission before any Embark request is
/// sent, so a running instance may claim as soon as Control starts it. A lost Embark
/// reply retries the same instance without a second admission.
pub(super) struct Launch {
    task: TaskId,
    deadline: u64,
    phase: LaunchPhase,
    pending: Option<Pending<Call>>,
}

impl Launch {
    pub(super) fn new(task: TaskId, deadline: u64) -> Self {
        Self {
            task,
            deadline,
            phase: LaunchPhase::Inspect,
            pending: None,
        }
    }

    pub(super) fn task(&self) -> TaskId {
        self.task
    }

    pub(super) fn returning(&mut self, trusted: bool) {
        self.pending = None;
        self.phase = LaunchPhase::Return { trusted };
    }

    pub(super) fn poll(&mut self, ledger: &mut Ledger, peers: &Peers, now: u64) -> LaunchReply {
        if let LaunchPhase::Return { trusted } = self.phase {
            return LaunchReply::Abort { trusted };
        }
        if self.pending.is_none() {
            return self.begin(ledger, peers, now);
        }
        let mut pending = self.pending.take().expect("checked pending launch");
        let said = match pending.poll() {
            Ok(Some(said)) => said,
            Ok(None) => {
                self.pending = Some(pending);
                return self.hold(ledger, now);
            }
            Err(_) => return self.retry(ledger, now),
        };
        let instance = peers.control.instance(self.task);
        match self.phase {
            LaunchPhase::Inspect => match instance.inspect_reply(said) {
                Ok(said) if suitable(&said) => match ledger.admit(self.task, self.deadline) {
                    Ok(()) => {
                        self.phase = LaunchPhase::Embark;
                        self.begin(ledger, peers, now)
                    }
                    Err(_) => LaunchReply::Abort { trusted: true },
                },
                Ok(_) => LaunchReply::Abort { trusted: true },
                Err(Fail::Unknown) | Err(Fail::Denied) => LaunchReply::Abort { trusted: false },
                Err(_) => self.retry(ledger, now),
            },
            LaunchPhase::Embark => match instance.embark_reply(said) {
                Ok(()) => LaunchReply::Started,
                Err(Fail::NotReady) | Err(Fail::Unknown) | Err(Fail::Denied) => {
                    LaunchReply::Abort { trusted: true }
                }
                Err(_) => self.retry(ledger, now),
            },
            LaunchPhase::Return { trusted } => LaunchReply::Abort { trusted },
        }
    }

    fn begin(&mut self, ledger: &Ledger, peers: &Peers, now: u64) -> LaunchReply {
        if now >= self.deadline {
            return LaunchReply::Abort {
                trusted: ledger.admission(self.task).is_some(),
            };
        }
        let instance = peers.control.instance(self.task);
        let wait = super::execution::budget(now, self.deadline);
        let pending = match self.phase {
            LaunchPhase::Inspect => instance.begin_inspect(wait),
            LaunchPhase::Embark => instance.begin_embark(wait),
            LaunchPhase::Return { trusted } => return LaunchReply::Abort { trusted },
        };
        match pending {
            Ok(pending) => {
                self.pending = Some(pending);
                LaunchReply::Pending
            }
            Err(Fail::Unknown) | Err(Fail::Denied) => LaunchReply::Abort {
                trusted: ledger.admission(self.task).is_some(),
            },
            Err(_) => LaunchReply::Pending,
        }
    }

    fn hold(&mut self, ledger: &Ledger, now: u64) -> LaunchReply {
        if now >= self.deadline {
            LaunchReply::Abort {
                trusted: ledger.admission(self.task).is_some(),
            }
        } else {
            LaunchReply::Pending
        }
    }

    fn retry(&mut self, ledger: &Ledger, now: u64) -> LaunchReply {
        self.hold(ledger, now)
    }
}

fn suitable(said: &system_api::control::frame::Said) -> bool {
    said.team.get() != 0
        && !matches!(
            State::of_code(said.a),
            Some(State::Stopping) | Some(State::Dead) | None
        )
}
