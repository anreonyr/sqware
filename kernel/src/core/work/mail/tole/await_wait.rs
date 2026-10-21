use alloc::sync::{Arc, Weak};
use core::time::Duration;

use env::{PieToken, ToleAwaitReply, ToleFail};

use crate::core::work::pend;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::core::work::unit::life::Life;
use crate::core::work::unit::task::Task;

use super::{ToleMeta, key};

#[derive(Clone)]
pub(crate) struct AwaitWait {
    pub group: Arc<ToleMeta>,
    pub token: PieToken,
    pub actor: Weak<Task>,
    pub deadline: Option<u64>,
}

fn inspect_key(actor: &Arc<Task>, token: PieToken) -> WakeKey {
    WakeKey::Inspect {
        task: actor.ident.id,
        token: token.get(),
    }
}

impl AwaitWait {
    pub fn new(group: Arc<ToleMeta>, token: PieToken, actor: &Arc<Task>, wait: env::Wait) -> Self {
        Self {
            group,
            token,
            actor: Arc::downgrade(actor),
            deadline: match wait {
                env::Wait::Forever => None,
                _ => Some(
                    crate::machine::clock::now()
                        .add(wait.into_duration())
                        .as_ticks(),
                ),
            },
        }
    }

    pub fn watch(&self) -> Result<(), ToleFail> {
        let actor = self.actor.upgrade().ok_or(ToleFail::Denied)?;
        messenger::forward(
            inspect_key(&actor, self.token),
            actor.life(),
            self.group.id,
            self.group.life(),
        )
        .map_err(|_| ToleFail::OoM)
    }

    pub fn unwatch(&self) {
        if let Some(actor) = self.actor.upgrade() {
            messenger::unforward(inspect_key(&actor, self.token), self.group.id);
        }
    }

    pub fn key(&self) -> WakeKey {
        key(&self.group)
    }

    pub fn life(&self) -> Weak<Life> {
        self.group.life()
    }

    pub fn poll(&self) -> Result<ToleAwaitReply, ToleFail> {
        let actor = self.actor.upgrade().ok_or(ToleFail::Denied)?;
        let pie =
            pend::accede(&actor, self.token, pend::Need::Fetch).map_err(super::codec::pie_fail)?;
        pend::available(&pie).map_err(super::codec::pie_fail)?;
        self.group.poll(&actor)
    }

    pub fn write(
        &self,
        frame: &mut crate::machine::switcher::context::TrapContext,
        result: Result<ToleAwaitReply, ToleFail>,
    ) {
        Self::write_result(frame, result);
    }

    pub fn write_result(
        frame: &mut crate::machine::switcher::context::TrapContext,
        result: Result<ToleAwaitReply, ToleFail>,
    ) {
        use crate::machine::switcher::context::Gprs;
        let words = match result {
            Ok(reply) => reply.words(),
            Err(e) => [env::FailCode::code(e) as usize, 0, 0],
        };
        frame.gpr.set_x(Gprs::A0, words[0]);
        frame.gpr.set_x(Gprs::A1, words[1]);
        frame.gpr.set_x(Gprs::A2, words[2]);
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.map_or(Duration::MAX, |at| {
            crate::machine::clock::ticks_to_duration(
                at.saturating_sub(crate::machine::clock::now().as_ticks()),
            )
        })
    }
}

unsafe impl Send for AwaitWait {}
unsafe impl Sync for AwaitWait {}
