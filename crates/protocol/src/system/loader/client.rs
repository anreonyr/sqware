use super::frame::{self, Ask, Fail, Said};
use crate::communication::{hand::Receiver, session::establish};
use crate::wire::message::Message;
use env::wire::Span;
use env::{Permission, PieToken, TaskId, TeamId, Wait, pie};
use runtime::core::res::pie::{HolePie};

#[derive(Clone, Copy, Debug)]
pub struct Built {
    pub team: TeamId,
    pub task: TaskId,
}
pub struct Face {
    entry: PieToken,
    host: TaskId,
}
impl Face {
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        Ok(Self {
            entry,
            host: establish::opened_by(entry).ok_or(Fail::Bad)?,
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
        let seed = pie::accord(image, self.host, Permission::FETCH, frame::IMAGE)
            .map_err(|_| Fail::Denied)?;
        let result = (|| {
            let (back, reply) =
                establish::lend_out(self.entry, frame::BACK).map_err(|_| Fail::Bad)?;
            let result = (|| {
                let mut words = [0; frame::MAX_ARGS];
                for (to, from) in words.iter_mut().zip(args) {
                    *to = *from as u64;
                }
                let ask = Ask {
                    op: frame::BUILD,
                    image: seed,
                    offset: offset as u64,
                    len: len as u64,
                    stack: stack as u64,
                    count: args.len() as u8,
                    args: words,
                    back: reply,
                };
                let mut bytes = [0; Ask::LEN];
                let n = ask.store_at(&mut bytes, 0).ok_or(Fail::Bad)?;
                HolePie::from_token(self.entry)
                    .push(&bytes[..n], remaining())
                    .map_err(|_| Fail::Bad)?;
                let mut buf = Said::EMPTY;
                let said = Receiver::<Said>::from_token(back)
                    .recv(&mut buf, remaining())
                    .map_err(|_| Fail::Bad)?;
                if said.status != crate::wire::OK {
                    return Err(crate::system::control::frame::code_to_fail(said.status)
                        .unwrap_or(Fail::Bad));
                }
                if said.team == 0 || said.task.get() == 0 {
                    return Err(Fail::Bad);
                }
                let confirm = pie::accord(back, self.host, Permission::STORE, frame::BACK)
                    .map_err(|_| Fail::Bad)?;
                let claim = frame::Claim {
                    op: frame::CLAIM,
                    task: said.task,
                    back: confirm,
                };
                let mut receipt = [0; frame::Claim::LEN];
                let n = claim.store_at(&mut receipt, 0).ok_or(Fail::Bad)?;
                HolePie::from_token(self.entry)
                    .push(&receipt[..n], remaining())
                    .map_err(|_| Fail::Bad)?;
                let confirmed = Receiver::<Said>::from_token(back)
                    .recv(&mut buf, remaining())
                    .map_err(|_| Fail::Bad)?;
                if confirmed.status != crate::wire::OK {
                    return Err(
                        crate::system::control::frame::code_to_fail(confirmed.status)
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
            })();
            let _ = pie::seal(back);
            let _ = pie::release(back);
            result
        })();
        let _ = pie::revoke(self.host, seed);
        result
    }
}
