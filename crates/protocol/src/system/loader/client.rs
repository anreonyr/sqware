use super::frame::{self, Ask, Fail, Said};
use crate::wire::message::Message;
use env::{Permission, PieToken, TaskId, TeamId, Wait};
use ::resource::{port::{Reply, Sender}, raw::Grant};

#[derive(Clone, Copy, Debug)]
pub struct Built {
    pub team: TeamId,
    pub task: TaskId,
}
pub struct Face {
    entry: Sender,
}
impl Face {
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        Ok(Self {
            entry: Sender::import(entry).map_err(|_| Fail::Bad)?,
        })
    }
    pub fn build(
        &self,
        image: PieToken,
        offset: usize,
        len: usize,
        args: &[usize],
        stack: usize,
        wait: Wait,
    ) -> Result<Built, Fail> {
        if args.len() > frame::MAX_ARGS
            || len == 0
            || len > frame::MAX_IMAGE
            || offset.checked_add(len).is_none()
        {
            return Err(Fail::Bad);
        }
        let started = env::chrono::clock();
        let remaining = || match wait {
            Wait::Forever => Wait::Forever,
            Wait::AtMost(ms) => Wait::AtMost(ms.saturating_sub(
                (env::chrono::clock().saturating_sub(started) / 1_000_000) as usize,
            )),
        };
        let image_grant = Grant::accord(&image, self.entry.peer(), Permission::FETCH, frame::IMAGE)
            .map_err(|_| Fail::Denied)?;
        let result = (|| {
            let back = Reply::open(self.entry.peer(), frame::BACK).map_err(|_| Fail::Bad)?;
            let reply = back.grant().map_err(|_| Fail::Bad)?;
            (|| {
                let mut words = [0; frame::MAX_ARGS];
                for (to, from) in words.iter_mut().zip(args) {
                    *to = *from as u64;
                }
                let ask = Ask {
                    op: frame::BUILD,
                    image: image_grant.remote(),
                    offset: offset as u64,
                    len: len as u64,
                    stack: stack as u64,
                    count: args.len() as u8,
                    args: words,
                    back: reply.remote(),
                };
                let mut bytes = [0; Ask::LEN];
                let n = frame::Wire::Build(ask).store(&mut bytes).ok_or(Fail::Bad)?;
                self.entry
                    .push(&bytes[..n], remaining())
                    .map_err(|_| Fail::Bad)?;
                let mut buf = Said::EMPTY;
                let said = Said::fetch(back.pull(&mut buf, remaining()).map_err(|_| Fail::Bad)?)
                    .ok_or(Fail::Bad)?;
                if said.status != crate::wire::OK {
                    return Err(frame::code_to_fail(said.status)
                        .unwrap_or(Fail::Bad));
                }
                if said.team == 0 || said.task.get() == 0 {
                    return Err(Fail::Bad);
                }
                let confirm = back.grant().map_err(|_| Fail::Bad)?;
                let claim = frame::Claim {
                    op: frame::CLAIM,
                    task: said.task,
                    back: confirm.remote(),
                };
                let mut receipt = [0; frame::Claim::LEN];
                let n = frame::Wire::Claim(claim).store(&mut receipt).ok_or(Fail::Bad)?;
                self.entry
                    .push(&receipt[..n], remaining())
                    .map_err(|_| Fail::Bad)?;
                let confirmed = Said::fetch(back.pull(&mut buf, remaining()).map_err(|_| Fail::Bad)?)
                    .ok_or(Fail::Bad)?;
                if confirmed.status != crate::wire::OK {
                    return Err(
                        frame::code_to_fail(confirmed.status)
                            .unwrap_or(Fail::Bad),
                    );
                }
                if confirmed.task != said.task || confirmed.team != said.team {
                    return Err(Fail::Bad);
                }
                Ok(Built {
                    team: TeamId::new(said.team as usize),
                    task: said.task,
                })
            })()
        })();
        result
    }
}
