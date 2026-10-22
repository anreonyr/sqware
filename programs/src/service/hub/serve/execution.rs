//! Bounded RPC work for Hub-owned execution instances.
//!
//! This adapter authenticates launch reports, asks Control to embark the authenticated
//! instance, and returns owner receipts once Hub has no further resource responsibility.
//! It keeps only in-flight RPC progress; running/death state stays with Control.

use alloc::collections::VecDeque;
use env::{TaskId, Wait};

use crate::service::hub::core::Ledger;

use super::launch::Launch;
use super::launch_reply::LaunchReply;
use super::peers::Peers;
use super::return_reply::ReturnReply;
use super::returned::Return;

const START_MS: u64 = 3_000_000_000;
pub(super) const RETURN_MS: usize = 3000;
const LAUNCH_CAP: usize = 256;
const RETURN_CAP: usize = 512;

pub(crate) struct Executions {
    launches: VecDeque<Launch>,
    returns: VecDeque<Return>,
    launch_cap: usize,
    return_cap: usize,
}

impl Executions {
    /// Reserve both bounded queues once, so later trusted handoff cannot fail on OOM.
    pub(crate) fn try_new() -> Result<Self, ()> {
        Self::with_capacity(LAUNCH_CAP, RETURN_CAP)
    }

    pub(crate) fn with_capacity(launch_cap: usize, return_cap: usize) -> Result<Self, ()> {
        let mut launches = VecDeque::new();
        launches.try_reserve(launch_cap).map_err(|_| ())?;
        let mut returns = VecDeque::new();
        returns.try_reserve(return_cap).map_err(|_| ())?;
        Ok(Self {
            launches,
            returns,
            launch_cap,
            return_cap,
        })
    }

    fn reserve_launch(&mut self) -> bool {
        self.launches.len() < self.launch_cap && self.launches.try_reserve(1).is_ok()
    }

    /// Accept one launch report if Hub has room and has not already settled its identity.
    pub(crate) fn receive(&mut self, ledger: &Ledger, task: TaskId, now: u64) -> bool {
        if ledger.admission(task).is_some() || ledger.admission_closed(task) || ledger.settling() {
            return true;
        }
        if self.launches.iter().any(|launch| launch.task() == task) {
            return true;
        }
        if !self.reserve_launch() {
            return false;
        }
        let deadline = now.saturating_add(START_MS);
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

    pub(crate) fn advance(&mut self, ledger: &mut Ledger, peers: &Peers, now: u64) {
        ledger.expire_admissions(now, |task| self.enqueue(task, true, now));
        for task in ledger.closed_admission_tasks() {
            self.enqueue(task, true, now);
        }

        let mut at = 0;
        while at < self.launches.len() {
            match self.launches[at].poll(ledger, peers, now) {
                LaunchReply::Pending => at += 1,
                LaunchReply::Started => {
                    self.launches.remove(at);
                }
                LaunchReply::Abort { trusted } => {
                    let launch = self.launches.remove(at).expect("launch index");
                    if trusted {
                        ledger.close_admission(launch.task());
                    }
                    self.enqueue(launch.task(), trusted, now);
                }
            }
        }

        let mut at = 0;
        while at < self.returns.len() {
            match self.returns[at].poll(peers, now) {
                ReturnReply::Pending => at += 1,
                ReturnReply::Released => {
                    let returned = self.returns.remove(at).expect("return index");
                    ledger.released(returned.task());
                }
                ReturnReply::NoReceipt => {
                    self.returns.remove(at);
                }
            }
        }

        if ledger.settling() {
            self.settle(ledger, now);
        }
    }

    /// The instance's resources are closing: request the owner handoff immediately.
    ///
    /// Hardware teardown still waits for Control's real per-instance target.
    pub(crate) fn closed(&mut self, ledger: &mut Ledger, task: TaskId, now: u64) -> bool {
        ledger.close_admission(task);
        self.enqueue(task, ledger.admission(task).is_some(), now)
    }

    /// A real target retirement proves this execution domain stopped.
    pub(crate) fn target(&mut self, ledger: &mut Ledger, task: TaskId, now: u64) -> bool {
        ledger.close_admission(task);
        self.enqueue(task, ledger.admission(task).is_some(), now)
    }

    pub(crate) fn settle(&mut self, ledger: &mut Ledger, now: u64) {
        let mut at = 0;
        while at < self.launches.len() {
            let task = self.launches[at].task();
            if self.enqueue(task, false, now) {
                self.launches.remove(at);
            } else {
                at += 1;
            }
        }
        for task in ledger.closed_admission_tasks() {
            self.enqueue(task, true, now);
        }
    }

    pub(crate) fn idle(&self, ledger: &Ledger) -> bool {
        self.launches.is_empty()
            && self.returns.is_empty()
            && ledger.admissions().next().is_none()
    }

    pub(crate) fn has_return(&self, task: TaskId) -> bool {
        self.returns.iter().any(|returned| returned.task() == task)
    }

    fn enqueue(&mut self, task: TaskId, trusted: bool, now: u64) -> bool {
        if let Some(returned) = self.returns.iter_mut().find(|item| item.task() == task) {
            if trusted {
                returned.trust();
            }
            return true;
        }
        if self.returns.len() >= self.return_cap || self.returns.try_reserve(1).is_err() {
            return false;
        }
        self.returns
            .push_back(Return::new(task, trusted, now.saturating_add(RETURN_MS as u64 * 1_000_000)));
        true
    }
}

/// Drain the bounded launch channel. A slot is reserved before the only copy is consumed.
pub(crate) fn receive(ledger: &mut Ledger, executions: &mut Executions) {
    for info in resource::raw::pies().filter(|info| {
        info.alive && info.kind == env::PieKind::Hole && info.mark == hub_api::LAUNCH_MARK
    }) {
        let mut bytes = [0u8; 8];
        loop {
            if !executions.reserve_launch() {
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
            if raw != 0 {
                let _ = executions.receive(ledger, TaskId::new(raw), env::chrono::clock());
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
