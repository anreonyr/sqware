use super::{receipt::Receipt, unit};
use crate::system::control::unit::{Control, table::State};
use env::TaskId;
use system_api::control::Fail;

impl Control {
    pub(crate) fn release_reported(
        &mut self,
        owner: TaskId,
        reference: crate::system::assembly::InstanceRef,
    ) -> bool {
        let Some(item) = self.instances.iter_mut().find(|item| {
            item.reference == reference && item.owner == owner && item.state == State::Dead
        }) else { return false };
        match item.receipt {
            Receipt::Consumed | Receipt::Reported => {
                item.receipt = Receipt::Reported;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn reclaim_returned(&mut self) {
        let mut consumed = false;
        for item in &mut self.instances {
            if item.state == State::Dead && matches!(item.receipt, Receipt::Returned(_))
                && matches!(item.receipt.consume(), Ok(true))
            {
                consumed = true;
            }
        }
        if consumed { self.reclaimed_at = env::chrono::clock(); }
    }

    pub(crate) fn release_instance(
        &mut self,
        owner: TaskId,
        task: TaskId,
    ) -> Result<Option<State>, Fail> {
        let item = self.instances.iter_mut().find(|item| item.task == task)
            .ok_or(Fail::Unknown)?;
        if item.owner != owner { return Err(Fail::Denied) }
        if let Receipt::Owner(team) = item.receipt {
            item.receipt = Receipt::Returned(team);
        }
        if item.state != State::Dead {
            item.stop();
            if let Some(team) = item.receipt.team() {
                let _ = unit::slay_team(team);
            }
            return Ok(None);
        }
        match item.receipt.consume() {
            Ok(consumed) => {
                if consumed { self.reclaimed_at = env::chrono::clock(); }
                Ok(Some(State::Dead))
            }
            Err(env::UnitFail::Busy) => Ok(None),
            Err(_) => Err(Fail::NotReady),
        }
    }
}
