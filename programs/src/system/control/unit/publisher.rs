use env::{Mark, TaskId};
use system_api::operator::Ops;

use super::table::{Slot, State};
use super::verdict::Fail;
use super::Control;

impl Control {
    /// Finds the single static program whose trusted UnitFile declares this
    /// namespace as a shared BIND authority for this resource mark.
    pub(crate) fn publisher(&self, namespace: Mark, resource: Mark) -> Result<TaskId, Fail> {
        if namespace.get() == 0 || resource.get() == 0 {
            return Err(Fail::Denied);
        }

        let mut expected: Option<&'static str> = None;
        for input in &self.inputs {
            let declares = input.program.operator.namespace.iter().any(|declaration| {
                declaration.key.mark() == namespace
                    && declaration.placement == crate::unit::operator::Placement::Shared
                    && declaration.ops.contains(Ops::BIND)
                    && declaration.source_mark == Some(resource.get())
            });
            if !declares {
                continue;
            }
            let name = input.program.name();
            match expected {
                Some(previous) if previous != name => return Err(Fail::Denied),
                Some(_) => {}
                None => expected = Some(name),
            }
        }

        let name = expected.ok_or(Fail::Unknown)?;
        let row = self.table.find(name).ok_or(Fail::Unknown)?;
        if row.state != State::Ready || !row.named || !row.run_requested {
            return Err(Fail::NotReady);
        }
        let task = match row.slot {
            Slot::Live { task, .. } => task,
            Slot::None => return Err(Fail::NotReady),
        };
        if task == TaskId::new(0) || !self.live(task) {
            return Err(Fail::NotReady);
        }
        Ok(task)
    }
}
