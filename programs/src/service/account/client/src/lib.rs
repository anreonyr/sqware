#![no_std]
//! Trusted Login requests construction under a configured account identity.
use ::resource::raw::reserve;
use account_api::ENTRY;
use account_api::{Call, DIR, Request};
use env::{PieToken, TaskId, TeamId, Wait, pie};
use ipc::{rpc, time::Deadline};
use system_api::control::{Fail, construction::CreateId};
use system_api::control::frame::{OK, code_to_fail};
use system_client::operator::Face;

#[derive(Clone, Copy, Debug)]
pub struct Built {
    pub task: TaskId,
    pub team: TeamId,
    pub startup: system_api::loader::StartupMetadata,
}
pub struct Client {
    entry: PieToken,
    host: TaskId,
}
impl Client {
    pub fn find(tree: &Face, wait: Wait) -> Result<Self, Fail> {
        let base = tree
            .resolve(system_api::namespace::SERVICE, wait)
            .map_err(|_| Fail::Bad)?;
        Self::of(
            tree.tile_from(base, DIR, wait)
                .and_then(|tile| tile.token(wait))
                .map_err(|_| Fail::Bad)?,
        )
    }
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = match reserve(entry) {
            Ok((_, host, mark)) if mark == ENTRY => host,
            _ => {
                let _ = pie::release(entry, env::ReleaseMode::Revoke);
                return Err(Fail::Bad);
            }
        };
        Ok(Self { entry, host })
    }
    pub fn create(&self, id: CreateId, account: &str, wait: Wait) -> Result<Built, Fail> {
        let deadline = Deadline::new(wait);
        let mut said = self.call(account_api::CREATE, id, account, deadline.remaining())?;
        while said.status == OK && said.state == system_api::control::construction::PENDING {
            let remaining = deadline.remaining();
            if remaining == Wait::POLL {
                return Err(Fail::NotReady);
            }
            said = self.call(account_api::INSPECT, id, account, remaining)?;
        }
        if said.state != system_api::control::construction::COMPLETED {
            return Err(if said.state == system_api::control::construction::FAILED {
                code_to_fail(said.status).unwrap_or(Fail::Bad)
            } else {
                Fail::NotReady
            });
        }
        self.built(said)
    }
    pub fn inspect(&self, id: CreateId, account: &str, wait: Wait) -> Result<system_api::loader::Said, Fail> {
        self.call(account_api::INSPECT, id, account, wait)
    }
    pub fn cancel(&self, id: CreateId, account: &str, wait: Wait) -> Result<system_api::loader::Said, Fail> {
        self.call(account_api::CANCEL, id, account, wait)
    }
    fn call(&self, op: u8, id: CreateId, account: &str, wait: Wait) -> Result<system_api::loader::Said, Fail> {
        let deadline = Deadline::new(wait);
        let sender = rpc::request::Sender::<Call>::from_raw(self.entry, Call::BACK)
            .map_err(|_| Fail::Bad)?;
        if sender.peer() != self.host {
            return Err(Fail::Bad);
        }
        let reply = sender
            .call(deadline, |back| Request {
                op,
                id,
                account: account.into(),
                back,
            })
            .map_err(|_| Fail::NotReady)?;
        Ok(reply)
    }
    fn built(&self, reply: system_api::loader::Said) -> Result<Built, Fail> {
        if reply.status != OK {
            return Err(code_to_fail(reply.status).unwrap_or(Fail::Bad));
        }
        if reply.task.get() == 0 || reply.team == 0 {
            return Err(Fail::Bad);
        }
        Ok(Built {
            task: reply.task,
            team: TeamId::new(reply.team as usize),
            startup: reply.startup,
        })
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = pie::release(self.entry, env::ReleaseMode::Revoke);
    }
}
