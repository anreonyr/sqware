use env::{PieToken, TaskId, Wait};

use ipc::rpc::request::Sender;
use ipc::session::establish;
use ipc::time::Deadline;

use super::Fail;
use super::client::deny;
use system_api::control::frame::{OK, code_to_fail};
use system_api::control::observe::{
    Call as ObserveCall, Reply as ObserveReply, Request as ObserveRequest,
};

/// Public supervision observation face.
pub struct Observe {
    sender: Sender<ObserveCall>,
    host: TaskId,
}

impl Observe {
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Bad)?;
        let sender =
            Sender::<ObserveCall>::from_raw(entry, ObserveCall::BACK).map_err(|_| Fail::Bad)?;
        if sender.peer() != host {
            return Err(Fail::Bad);
        }
        Ok(Observe { sender, host })
    }

    pub fn host(&self) -> TaskId {
        self.host
    }

    pub fn begin_observe(
        &self,
        index: u32,
        wait: Wait,
    ) -> Result<ipc::rpc::request::Pending<ObserveCall>, Fail> {
        self.sender
            .begin(Deadline::new(wait), move |back| ObserveRequest { index, back })
            .map_err(|_| deny("observe"))
    }

    pub fn poll_observe(
        &self,
        pending: &mut ipc::rpc::request::Pending<ObserveCall>,
    ) -> Result<Option<ObserveReply>, Fail> {
        match pending.poll().map_err(|_| deny("observe"))? {
            Some(reply) if reply.status == OK => Ok(Some(reply)),
            Some(reply) => Err(code_to_fail(reply.status).unwrap_or(Fail::Bad)),
            None => Ok(None),
        }
    }
}
