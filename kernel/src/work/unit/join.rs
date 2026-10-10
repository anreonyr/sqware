use super::team::{Member, TeamLife, TeamState};
use crate::runtime::chrono::clock;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use alloc::sync::Arc;
use core::time::Duration;
use env::{JoinReply, TaskId, UnitFail, Wait};

#[derive(Clone)]
pub(crate) enum JoinTarget {
    Task(Arc<Member>),
    Team(Arc<TeamLife>),
}
#[derive(Clone)]
pub(crate) struct JoinWait {
    pub target: JoinTarget,
    pub owner: TaskId,
    pub receive: bool,
    pub deadline: Option<u64>,
}
impl JoinWait {
    pub fn new(target: JoinTarget, owner: TaskId, receive: bool, wait: Wait) -> Self {
        Self {
            target,
            owner,
            receive,
            deadline: match wait {
                Wait::Forever => None,
                _ => Some(clock::now().add(wait.into_duration()).as_ticks()),
            },
        }
    }
    pub fn remaining(&self) -> Duration {
        self.deadline.map_or(Duration::MAX, |at| {
            clock::ticks_to_duration(at.saturating_sub(clock::now().as_ticks()))
        })
    }
    pub fn key(&self) -> crate::work::room::messenger::WakeKey {
        match &self.target {
            JoinTarget::Task(m) => crate::work::room::messenger::WakeKey::Task { id: m.id },
            JoinTarget::Team(t) => crate::work::room::messenger::WakeKey::Team { id: t.id },
        }
    }
    pub fn life(&self) -> alloc::sync::Weak<super::life::Life> {
        match &self.target {
            JoinTarget::Task(m) => Arc::downgrade(&m.life),
            JoinTarget::Team(t) => Arc::downgrade(&t.life),
        }
    }
    pub fn poll(&self) -> Result<JoinReply, UnitFail> {
        let _commit = super::commit();
        let member = match &self.target {
            JoinTarget::Task(m) => {
                if self.receive && !m.pending(self.owner) {
                    return Err(UnitFail::Denied);
                }
                Some(m.clone())
            }
            JoinTarget::Team(t) => {
                if t.owner != self.owner || matches!(&*t.state.lock(), TeamState::Ousted) {
                    return Err(UnitFail::Denied);
                }
                t.next(self.owner)
            }
        };
        let Some(member) = member else {
            return Ok(JoinReply::Pending);
        };
        let Some(exit) = member.exit() else {
            return Ok(JoinReply::Pending);
        };
        if self.receive {
            if !member.receive(self.owner) {
                return Err(UnitFail::Denied);
            }
            if let Some(node) = member.node.upgrade() {
                let mut root = node;
                while let Some(parent) = &root.parent {
                    root = parent.clone();
                }
                root.prune();
            }
        }
        Ok(JoinReply::Reaped(exit))
    }
    pub fn write(frame: &mut TrapContext, result: Result<JoinReply, UnitFail>) {
        let words = match result {
            Ok(JoinReply::Pending) => [0, 0, 0],
            Ok(JoinReply::Reaped(exit)) => [exit.cause as usize, exit.task.get(), exit.reason],
            Err(error) => [env::FailCode::code(error) as usize, 0, 0],
        };
        frame.gpr.set_x(Gprs::A0, words[0]);
        frame.gpr.set_x(Gprs::A1, words[1]);
        frame.gpr.set_x(Gprs::A2, words[2]);
    }
}

/// Shared authorization for Join and non-consuming Mail observations.
pub(crate) fn observe(
    me: &Arc<super::task::Task>,
    target: env::UnitTarget,
) -> Result<JoinTarget, UnitFail> {
    let _commit = super::commit();
    match target {
        env::UnitTarget::Team(id) => me
            .heir(id)
            .map(|t| JoinTarget::Team(t.life.clone()))
            .ok_or(UnitFail::Denied),
        env::UnitTarget::Task(id) => {
            let mut member = me.ident.team.life.member(id);
            let mut at = 0;
            while member.is_none() {
                let Some(root) = me.heir_node(at) else { break };
                at += 1;
                member = root
                    .life
                    .member(id)
                    .or_else(|| root.life.find(id).filter(|m| m.pending(me.ident.id)));
            }
            member.map(JoinTarget::Task).ok_or(UnitFail::Denied)
        }
    }
}
