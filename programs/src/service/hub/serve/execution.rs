//! Bounded RPC work for Hub-owned execution instances.
//!
//! This adapter authenticates launch reports, asks Control to embark the authenticated
//! instance, and returns owner receipts once Hub has no further resource responsibility.
//! It keeps only in-flight RPC progress; running/death state stays with Control.

use alloc::collections::VecDeque;
use env::{TaskId, Wait};

use crate::service::hub::core::Ledger;

use super::launch::{self, Launch};
use super::peers::Peers;
use super::returned::{self, Return};

const START_MS: u64 = 3_000_000_000;
pub(super) const RETURN_MS: usize = 3000;
const LAUNCH_CAP: usize = 256;
const RETURN_CAP: usize = 256;

pub(crate) struct Executions {
    launches: VecDeque<Launch>,
    returns: VecDeque<Return>,
}

impl Executions {
    pub(crate) fn new() -> Self {
        Self {
            launches: VecDeque::new(),
            returns: VecDeque::new(),
        }
    }

    /// Accept one launch report if Hub has room and has not already settled its identity.
    pub(crate) fn receive(&mut self, ledger: &Ledger, task: TaskId) -> bool {
        if ledger.admission(task).is_some() || ledger.admission_closed(task) {
            return true;
        }
        if ledger.settling() {
            return true;
        }
        if self.launches.iter().any(|launch| launch.task() == task) {
            return true;
        }
        if self.launches.len() >= LAUNCH_CAP || self.launches.try_reserve(1).is_err() {
            return false;
        }
        let deadline = env::chrono::clock().saturating_add(START_MS);
        self.launches.push_back(Launch::new(task, deadline));
        true
    }

    pub(crate) fn pending(&self, ledger: &Ledger) -> bool {
        !self.launches.is_empty()
            || !self.returns.is_empty()
            || ledger
                .admissions()
                .any(|admission| !admission.closed() && admission.claim_high() == 0)
    }

    pub(crate) fn advance(&mut self, ledger: &mut Ledger, peers: &Peers) {
        let now = env::chrono::clock();
        ledger.expire_admissions(now, |task| {
            self.enqueue(task, true, now);
        });

        let mut at = 0;
        while at < self.launches.len() {
            match self.launches[at].poll(peers, now) {
                launch::Reply::Pending => at += 1,
                launch::Reply::Admitted => {
                    let launch = self.launches.remove(at).expect("launch index");
                    if ledger.admit(launch.task(), launch.deadline()).is_err() {
                        self.enqueue(launch.task(), true, now);
                    }
                }
                launch::Reply::Abort { trusted } => {
                    let launch = self.launches.remove(at).expect("launch index");
                    self.enqueue(launch.task(), trusted, now);
                }
            }
        }

        let mut at = 0;
        while at < self.returns.len() {
            match self.returns[at].poll(peers, now) {
                returned::Reply::Pending => at += 1,
                returned::Reply::Released => {
                    let returned = self.returns.remove(at).expect("return index");
                    ledger.released(returned.task());
                }
                returned::Reply::NoReceipt => {
                    self.returns.remove(at);
                }
            }
        }

        if ledger.settling() {
            for task in ledger.admission_tasks() {
                if ledger.open_bindings_for(task) == 0 {
                    self.enqueue(task, true, now);
                }
            }
        }
    }

    /// The instance's resources are closed: return its receipt once nothing remains bound.
    pub(crate) fn closed(&mut self, ledger: &Ledger, task: TaskId) {
        if ledger.open_bindings_for(task) == 0 {
            self.enqueue(task, ledger.admission(task).is_some(), env::chrono::clock());
        }
    }

    /// A real target retirement proves this execution domain stopped.
    pub(crate) fn target(&mut self, ledger: &Ledger, task: TaskId) {
        self.launches.retain(|launch| launch.task() != task);
        if ledger.open_bindings_for(task) == 0 {
            self.enqueue(task, ledger.admission(task).is_some(), env::chrono::clock());
        }
    }

    pub(crate) fn settle(&mut self, ledger: &mut Ledger) {
        let tasks: alloc::vec::Vec<TaskId> =
            self.launches.iter().map(|launch| launch.task()).collect();
        self.launches.clear();
        for task in tasks {
            self.enqueue(task, false, env::chrono::clock());
        }
        let now = env::chrono::clock();
        for task in ledger.admission_tasks() {
            if ledger.open_bindings_for(task) == 0 {
                self.enqueue(task, true, now);
            }
        }
    }

    pub(crate) fn idle(&self, ledger: &Ledger) -> bool {
        self.launches.is_empty()
            && self.returns.is_empty()
            && ledger.admissions().next().is_none()
    }

    fn enqueue(&mut self, task: TaskId, trusted: bool, now: u64) {
        if let Some(returned) = self.returns.iter_mut().find(|item| item.task() == task) {
            if trusted {
                returned.trust();
            }
            return;
        }
        if self.returns.len() >= RETURN_CAP || self.returns.try_reserve(1).is_err() {
            return;
        }
        self.returns
            .push_back(Return::new(task, trusted, now.saturating_add(RETURN_MS as u64 * 1_000_000)));
    }
}

/// Drain the bounded launch channel. A full queue leaves the request for the next tick.
pub(crate) fn receive(ledger: &mut Ledger, executions: &mut Executions) {
    for info in resource::raw::pies().filter(|info| {
        info.alive && info.kind == env::PieKind::Hole && info.mark == hub_api::LAUNCH_MARK
    }) {
        let mut bytes = [0u8; 8];
        loop {
            if executions.launches.len() >= LAUNCH_CAP {
                return;
            }
            let Ok((len, _)) = resource::raw::Hole::from_raw(info.token).pull(&mut bytes, Wait::POLL)
            else {
                break;
            };
            if len != bytes.len() {
                continue;
            }
            let Ok(raw) = usize::try_from(u64::from_le_bytes(bytes)) else {
                continue;
            };
            if raw != 0 && !executions.receive(ledger, TaskId::new(raw)) {
                return;
            }
        }
    }
}

pub(super) fn budget(now: u64, deadline: u64) -> Wait {
    if now >= deadline {
        Wait::POLL
    } else {
        Wait::AtMost(((deadline - now).div_ceil(1_000_000)).max(1) as usize)
    }
}
