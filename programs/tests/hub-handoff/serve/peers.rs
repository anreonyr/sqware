use env::{TaskId, Wait};
use system_api::control::{Call, Fail, frame::Said};

use crate::backend::{self, Kind};
use crate::rpc::request::Pending;

pub struct Peers {
    pub control: ControlFace,
}

pub struct ControlFace;

impl ControlFace {
    pub fn instance(&self, task: TaskId) -> Instance {
        Instance { task }
    }
}

pub struct Instance {
    task: TaskId,
}

impl Instance {
    pub fn begin_inspect(&self, _: Wait) -> Result<Pending<Call>, Fail> {
        backend::begin(Kind::Inspect);
        Ok(Pending::new(Kind::Inspect))
    }

    pub fn begin_embark(&self, _: Wait) -> Result<Pending<Call>, Fail> {
        backend::begin(Kind::Embark);
        Ok(Pending::new(Kind::Embark))
    }

    pub fn begin_release(&self, _: Wait) -> Result<Pending<Call>, Fail> {
        backend::begin(Kind::Release);
        Ok(Pending::new(Kind::Release))
    }

    pub fn inspect_reply(&self, said: Said) -> Result<Said, Fail> {
        let said = backend::reply(said).map(|()| said)?;
        if said.task != self.task || system_api::control::State::of_code(said.a).is_none() {
            return Err(Fail::Bad);
        }
        Ok(said)
    }

    pub fn embark_reply(&self, said: Said) -> Result<(), Fail> {
        backend::reply(said)
    }

    pub fn release_reply(&self, said: Said) -> Result<(), Fail> {
        backend::reply(said)
    }
}
