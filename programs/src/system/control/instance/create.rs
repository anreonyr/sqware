//! Reserve lifecycle capacity before invoking the builder, then publish once.
use super::state::{self as instance, Instance};
use crate::system::control::unit::{Control, table::State};
use env::TaskId;
use system_api::{control::Fail, loader::Built};

impl Control {
    pub(crate) fn create_instance(
        &mut self,
        owner: TaskId,
        construct: impl FnOnce() -> Result<Built, Fail>,
    ) -> Result<Built, Fail> {
        self.reserve_instance()?;
        let built = construct()?;
        self.register_instance(built, owner);
        Ok(built)
    }
    fn reserve_instance(&mut self) -> Result<(), Fail> {
        if self.instances.len() >= instance::INSTANCE_CAP {
            if let Some(at) = self
                .instances
                .iter()
                .position(|item| item.state == State::Dead)
            {
                self.instances.remove(at);
            } else {
                return Err(Fail::Full);
            }
        }
        self.instances.try_reserve(1).map_err(|_| Fail::Full)
    }

    fn register_instance(&mut self, built: Built, owner: TaskId) {
        self.instances.push(Instance {
            owner,
            task: built.task,
            team: Some(built.team),
            state: State::Starting,
            claimed: false,
            started: false,
            reason: None,
            claim_until: env::chrono::clock() + system_api::loader::CLAIM_MS as u64 * 1_000_000,
            hook: Default::default(),
        });
    }
}
