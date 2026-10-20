use env::TaskId;
use hub_api::Fail;

use super::{Execution, ExecutionStep, Ledger};

const EXECUTIONS: usize = 256;

impl Ledger {
    pub fn reserve_execution(&mut self) -> Result<(), Fail> {
        if self.executions.len() >= EXECUTIONS {
            return Err(Fail::Full);
        }
        self.executions.try_reserve(1).map_err(|_| Fail::Full)
    }

    pub fn launch(&mut self, task: TaskId, deadline: u64) -> Result<(), Fail> {
        if self.executions.iter().any(|execution| execution.task == task) {
            return Ok(());
        }
        if self.settling {
            return Err(Fail::Denied);
        }
        self.reserve_execution()?;
        self.executions.push(Execution::new(task, deadline));
        Ok(())
    }

    pub fn executions(&mut self) -> &mut [Execution] {
        &mut self.executions
    }

    pub fn execution(&mut self, task: TaskId) -> Option<&mut Execution> {
        self.executions.iter_mut().find(|execution| execution.task == task)
    }

    pub fn executions_pending(&self) -> bool {
        self.executions.iter().any(Execution::pending)
    }

    pub fn prune_executions(&mut self) {
        self.executions.retain(|execution| {
            execution.step != ExecutionStep::Done
                || self.bindings.iter().any(|binding| {
                    binding.open() && binding.execution == execution.task
                })
        });
    }
}
