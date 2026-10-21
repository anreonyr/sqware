//! Reserve lifecycle capacity before invoking the builder, then publish once.
use super::state::{self as instance, Instance};
use crate::system::control::instance::receipt::Receipt;
use crate::system::control::unit::{Control, table::State};
use env::TaskId;
use system_api::{control::Fail, loader::Built};

pub(crate) struct Admission {
    pub owner: TaskId,
    pub job: crate::system::assembly::JobId,
}

impl Control {
    pub(crate) fn create_instance(
        &mut self,
        admission: Admission,
        construct: impl FnOnce() -> Result<Built, Fail>,
    ) -> Result<Built, Fail> {
        self.reserve_instance()?;
        let built = construct()?;
        self.register_instance(built, admission);
        Ok(built)
    }
    fn reserve_instance(&mut self) -> Result<(), Fail> {
        if self.instances.len() >= instance::INSTANCE_CAP {
            if let Some(at) = self
                .instances
                .iter()
                .position(|item| item.state == State::Dead && item.receipt == Receipt::Consumed)
            {
                self.instances.remove(at);
            } else {
                return Err(Fail::Full);
            }
        }
        self.instances.try_reserve(1).map_err(|_| Fail::Full)
    }

    fn register_instance(&mut self, built: Built, admission: Admission) {
        self.instances.push(Instance {
            reference: crate::system::assembly::InstanceRef {
                job: admission.job,
                task: built.task,
            },
            owner: admission.owner,
            task: built.task,
            receipt: Receipt::Owner(built.team),
            state: State::Starting,
            claimed: false,
            started: false,
            reason: None,
            claim_until: 0,
        });
    }
}
