#![no_std]
//! Trusted Login requests construction under a configured account identity.
use ::resource::raw::reserve;
use account_api::ENTRY;
use account_api::{Call, DIR, Request};
use env::{PieToken, TaskId, TeamId, Wait, pie};
use ipc::{rpc, time::Deadline};
use system_api::control::Fail;
use system_api::control::frame::{OK, code_to_fail};
use system_client::{loader::Built, operator::Face};
pub struct Client {
    entry: PieToken,
    host: TaskId,
}
impl Client {
    pub fn find(tree: &Face, wait: Wait) -> Result<Self, Fail> {
        Self::of(
            tree.tile(DIR, wait)
                .and_then(|tile| tile.token(wait))
                .map_err(|_| Fail::Bad)?,
        )
    }
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = match reserve(entry) {
            Ok((_, host, mark)) if mark == ENTRY => host,
            _ => {
                let _ = pie::release(entry);
                return Err(Fail::Bad);
            }
        };
        Ok(Self { entry, host })
    }
    pub fn create(&self, account: &str, wait: Wait) -> Result<Built, Fail> {
        let deadline = Deadline::new(wait);
        let sender = rpc::request::Sender::<Call>::from_raw(self.entry, Call::BACK)
            .map_err(|_| Fail::Bad)?;
        if sender.peer() != self.host {
            return Err(Fail::Bad);
        }
        let reply = sender
            .call(deadline, |back| Request {
                account: account.into(),
                back,
            })
            .map_err(|_| Fail::Bad)?;
        if reply.status != OK {
            return Err(code_to_fail(reply.status).unwrap_or(Fail::Bad));
        }
        if reply.task.get() == 0 || reply.team == 0 {
            return Err(Fail::Bad);
        }
        Ok(Built {
            task: reply.task,
            team: TeamId::new(reply.team as usize),
        })
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = pie::release(self.entry);
    }
}
