use env::{TaskId, ToleEventSource, Wait};

use crate::Pipe;

pub struct Creating;

impl Creating {
    pub fn poll(&mut self) -> Result<Option<Pipe>, pipe_api::Fail> {
        Ok(None)
    }

    pub fn source(&self) -> ToleEventSource {
        ToleEventSource::Join {
            target: TaskId::new(1).into(),
        }
    }

    pub fn remaining(&self) -> Wait {
        Wait::AtMost(1)
    }
}
