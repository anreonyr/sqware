use crate::work::unit::task::Task;
use alloc::sync::Arc;
use env::{PieToken, TaskId};

/// Resolve the current parent under both task gates. No task roster is scanned.
pub(crate) fn vestor(task: &Arc<Task>, token: PieToken) -> Option<TaskId> {
    for _ in 0..super::RETRIES {
        let observed = {
            let _gate = task.gate.lock();
            super::locate(task, token)?
        };
        let parent = observed.sire()?;
        let lord = observed.lord().upgrade()?;
        let result = super::with_pair(task, &lord, |_, _| {
            let Some(current) = super::locate(task, token) else {
                return Some(None);
            };
            if current.sire() != Some(parent) || !current.lord().ptr_eq(observed.lord()) {
                return None;
            }
            Some(super::locate(&lord, parent).map(|_| lord.ident.id))
        });
        if let Some(result) = result {
            return result;
        }
    }
    None
}
