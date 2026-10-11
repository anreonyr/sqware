use alloc::{format, string::String};
use env::{PieToken, TaskId, Wait};
use ipc::{
    rpc::request::{Pending, Sender},
    time::Deadline,
};
use shell_api::Image;
use system_api::loader;
pub const WAIT: Wait = Wait::AtMost(5000);
enum Phase {
    Build(Pending<loader::Call>),
    Claim(Pending<loader::Call>),
    Done,
}
pub struct Loading {
    sender: Sender<loader::Call>,
    deadline: Deadline,
    phase: Phase,
    image: Option<(TaskId, PieToken)>,
    pub built: Option<loader::Built>,
}
impl Loading {
    pub fn begin(entry: PieToken, image: &Image, owner: TaskId) -> Result<Self, String> {
        let sender = Sender::<loader::Call>::from_raw(entry, loader::Call::BACK)
            .map_err(|e| format!("loader entry: {e:?}"))?;
        let peer = sender.peer();
        let deadline = Deadline::new(WAIT);
        let loan = env::pie::accord(image.seed, peer, env::Permission::FETCH, loader::IMAGE)
            .map_err(|e| format!("image grant: {e:?}"))?;
        let pending = match sender.begin(deadline, |back| {
            let mut args = [0; loader::MAX_ARGS];
            args[0] = owner.get() as u64;
            loader::Wire::Build(loader::Ask {
                op: loader::BUILD,
                image: loan,
                offset: 0,
                len: image.length as u64,
                stack: 0,
                count: 1,
                args,
                back,
            })
        }) {
            Ok(pending) => pending,
            Err(error) => {
                let _ = env::pie::revoke(peer, loan);
                return Err(format!("loader build: {error:?}"));
            }
        };
        Ok(Self {
            sender,
            deadline,
            phase: Phase::Build(pending),
            image: Some((peer, loan)),
            built: None,
        })
    }
    pub fn source(&self) -> Option<env::Source> {
        match &self.phase { Phase::Build(p) | Phase::Claim(p) => Some(p.source()), Phase::Done => None }
    }
    pub fn remaining(&self) -> Wait { self.deadline.remaining() }
    pub fn poll(&mut self) -> Result<Option<loader::Built>, String> {
        let result = match &mut self.phase {
            Phase::Build(pending) | Phase::Claim(pending) => pending.poll(),
            Phase::Done => return Ok(self.built),
        };
        let Some(reply) = result.map_err(|error| format!("loader transport: {error:?}"))? else {
            return Ok(None);
        };
        if reply.status != 0 {
            return Err(format!("loader rejected: {}", reply.status));
        }
        if reply.task.get() == 0 || reply.team == 0 {
            return Err("invalid loader result".into());
        }
        match self.phase {
            Phase::Build(_) => {
                let built = loader::Built {
                    task: reply.task,
                    team: env::TeamId::new(reply.team as usize),
                };
                self.built = Some(built);
                if let Some((peer, token)) = self.image.take() {
                    let _ = env::pie::revoke(peer, token);
                }
                self.phase = Phase::Claim(
                    self.sender
                        .begin(self.deadline, |back| {
                            loader::Wire::Claim(loader::Claim {
                                op: loader::CLAIM,
                                task: built.task,
                                back,
                            })
                        })
                        .map_err(|e| format!("loader claim: {e:?}"))?,
                );
                Ok(None)
            }
            Phase::Claim(_) => {
                let built = self.built.ok_or("loader claim has no instance")?;
                if reply.task != built.task || reply.team != built.team.get() as u64 {
                    return Err("loader claim mismatch".into());
                }
                self.phase = Phase::Done;
                Ok(Some(built))
            }
            Phase::Done => Ok(self.built),
        }
    }
}
impl Drop for Loading {
    fn drop(&mut self) {
        if let Some((peer, token)) = self.image.take() {
            let _ = env::pie::revoke(peer, token);
        }
    }
}
