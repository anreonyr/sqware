use env::{TaskId, Wait};
use system_api::control::{Call, Fail, frame::Said};

use crate::rpc::request::Pending;

pub struct Instance {
    pub task: TaskId,
}

impl Instance {
    pub fn begin_release(
        &self,
        _: Wait,
    ) -> Result<Pending<Call>, Fail> {
        crate::backend::begin_release();
        Ok(Pending::new())
    }

    pub fn release_reply(&self, said: Said) -> Result<(), Fail> {
        match system_api::control::frame::code_to_fail(said.status) {
            None if said.status == system_api::control::frame::OK => Ok(()),
            Some(fail) => Err(fail),
            None => Err(Fail::Bad),
        }
    }
}
