use alloc::rc::Rc;
use core::cell::RefCell;

pub use crate::attach_request::AttachRequest;
use crate::recovery_api::Fail;
use super::{action::Action, script::Script};

pub struct Core {
    shared: Rc<RefCell<Script>>,
}

impl Core {
    pub fn new(shared: Rc<RefCell<Script>>) -> Self {
        Self { shared }
    }

    pub fn attach(&mut self, _: AttachRequest) -> Result<(), Fail> {
        let mut script = self.shared.borrow_mut();
        script.action(Action::CoreAttach);
        if script.attach_ok {
            Ok(())
        } else {
            Err(Fail::Denied)
        }
    }

    pub fn stop(&mut self, _: u64) -> Result<u8, Fail> {
        let mut script = self.shared.borrow_mut();
        script.action(Action::CoreStop);
        if script.stop_ok {
            Ok(0)
        } else {
            Err(Fail::Unavailable)
        }
    }

    pub fn detach(&mut self, _: u64) -> Result<(), Fail> {
        let mut script = self.shared.borrow_mut();
        script.action(Action::CoreDetach);
        if script.detach_ok {
            Ok(())
        } else {
            Err(Fail::Unavailable)
        }
    }
}
