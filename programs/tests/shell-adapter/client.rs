use env::{TaskId, Wait};
use pipe_api::Direction;

use crate::{Creating, Pipe, Port};

pub struct Client;

impl Client {
    pub fn begin_create(&self, _: usize, _: Wait) -> Result<Creating, pipe_api::Fail> {
        Ok(Creating)
    }

    pub fn begin_bind(
        &self,
        _: &Pipe,
        _: Direction,
        _: TaskId,
        _: Wait,
    ) -> Result<crate::rpc::request::Pending<pipe_api::Call>, pipe_api::Fail> {
        Ok(crate::rpc::request::Pending::new())
    }

    pub fn import(&self, _: pipe_api::Endpoint, _: Wait) -> Result<Port, pipe_api::Fail> {
        Ok(Port)
    }

    pub fn begin_release(
        &self,
        _: &Pipe,
        _: Wait,
    ) -> Result<crate::rpc::request::Pending<pipe_api::Call>, pipe_api::Fail> {
        Ok(crate::rpc::request::Pending::new())
    }
}
