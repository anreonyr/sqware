use env::{PieToken, TaskId, ToleEventSource, Wait};
use system_api::{control::construction, identity::Subject, loader};

pub const WAIT: Wait = Wait::AtMost(5000);

pub struct Loading {
    pub built: Option<loader::Built>,
}

impl Loading {
    pub fn begin(
        _: PieToken,
        _: PieToken,
        _: &shell_api::Image,
        _: TaskId,
        _: Subject,
        _: TaskId,
        _: construction::CreateId,
    ) -> Result<Self, alloc::string::String> {
        Ok(Self { built: None })
    }

    pub fn poll(&mut self) -> Result<Option<loader::Built>, alloc::string::String> {
        Ok(None)
    }

    pub fn done(&self) -> bool {
        true
    }

    pub fn cancel(&mut self) -> Result<(), alloc::string::String> {
        Ok(())
    }

    pub fn source(&self) -> Option<ToleEventSource> {
        None
    }

    pub fn remaining(&self) -> Wait {
        WAIT
    }
}
