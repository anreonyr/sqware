//! Trusted Login requests construction under a configured account identity.
use super::Fail;
use crate::system::{loader::Built, operator::Face};
use ::resource::raw::reserve;
use env::{PieToken, TaskId, TeamId, Wait, pie};
use ipc::{rpc, time::Deadline};
pub use system_api::control::account::{BACK, DIR, ENTRY};
pub mod frame;
pub use frame::Request;
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
        let sender = rpc::request::Sender::<super::rpc::Account>::from_raw(self.entry)
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
        if reply.status != super::frame::OK {
            return Err(super::frame::code_to_fail(reply.status).unwrap_or(Fail::Bad));
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
