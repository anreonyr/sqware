use env::{PieToken, TaskId, Wait};
use ipc::rpc::request::{Pending, Sender};
use ipc::time::Deadline;
use system_api::retirement::{Call, Retirement};

pub struct Face {
    sender: Sender<Call>,
}

impl Face {
    pub fn of(entry: PieToken) -> Result<Self, ipc::rpc::Fail> {
        Ok(Self {
            sender: Sender::from_raw(entry, Call::BACK)?,
        })
    }

    pub fn peer(&self) -> TaskId {
        self.sender.peer()
    }

    pub fn begin(&self, within: Wait) -> Result<Pending<Call>, ipc::rpc::Fail> {
        self.sender
            .begin(Deadline::new(within), |back| Retirement::ask(back))
    }
}
