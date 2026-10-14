use crate::machine::clock;
use crate::machine::switcher::context::{Gprs, TrapContext};
use alloc::sync::Arc;
use core::time::Duration;
use env::{JoinReply, TaskId, UnitFail, Wait};

mod target;
pub(crate) use target::JoinTarget;

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
    pub fn poll(&self) -> Result<JoinReply, UnitFail> {
        let _commit = crate::core::work::commit();
        let member = match &self.target {
            JoinTarget::Task(m) => {
                if self.receive && !m.pending(self.owner) {
                    return Err(UnitFail::Denied);
                }
                Some(m.clone())
            }
            JoinTarget::Team(t) => {
                if !self.receive {
                    let constructing = t.state.lock().constructing();
                    return Ok(if !constructing && t.all_reaped() {
                        JoinReply::Complete
                    } else {
                        JoinReply::Pending
                    });
                }
                let mut node = Some(&**t);
                let mut qualified = false;
                while let Some(here) = node {
                    qualified |= here.owner == self.owner;
                    node = here.parent.as_deref();
                }
                if !qualified {
                    return Err(UnitFail::Denied);
                }
                let next = t.next(self.owner);
                if next.is_none() && t.state.lock().phase.ousted() {
                    return Err(UnitFail::Denied);
                }
                next
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
            Ok(JoinReply::Complete) => [5, 0, 0],
            Ok(JoinReply::Reaped(exit)) => [exit.cause as usize, exit.task.get(), exit.reason],
            Err(error) => [env::FailCode::code(error) as usize, 0, 0],
        };
        frame.gpr.set_x(Gprs::A0, words[0]);
        frame.gpr.set_x(Gprs::A1, words[1]);
        frame.gpr.set_x(Gprs::A2, words[2]);
    }
}

pub(crate) fn observe(
    me: &Arc<super::task::Task>,
    target: env::UnitTarget,
) -> Result<JoinTarget, UnitFail> {
    let _commit = crate::core::work::commit();
    match target {
        env::UnitTarget::Team(id) => {
            if id == me.ident.team.id {
                return Err(UnitFail::Denied);
            }
            me.ident.team.life.find_team(id)
                .map(JoinTarget::Team)
                .ok_or(UnitFail::Denied)
        }
        env::UnitTarget::Task(id) => {
            let member = me
                .ident
                .team
                .life
                .member(id)
                .or_else(|| me.ident.team.life.find(id));
            member.map(JoinTarget::Task).ok_or(UnitFail::Denied)
        }
    }
}
