//! Register a constructed, paused instance; rejected delivery is reclaimed.
use crate::system::control::unit::Control;
use env::TaskId;
use system_api::{control::Fail, loader::Built};

impl Control {
    pub(crate) fn create_instance(&mut self, built: Built, owner: TaskId) -> Result<Built, Fail> {
        if let Err(fail) = self.reserve_instance().and_then(|_| env::unit::observe(built.team).map_err(|_| Fail::Denied)) {
            let _ = env::room::doom(built.task);
            let _ = env::unit::oust(built.team);
            return Err(fail);
        }
        self.register_instance(built, owner);
        Ok(built)
    }
}
