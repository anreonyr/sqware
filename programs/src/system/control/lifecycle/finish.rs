use super::{Action, Origin, Tracked};
use crate::system::control::unit::verdict::Fail;

impl Tracked {
    pub(crate) fn finish(
        &mut self,
        result: Result<::schedule::Progress, Fail>,
    ) -> Result<(), crate::system::app::Fault> {
        match result {
            Ok(::schedule::Progress::Done) => {
                let action = self.operation.key.request.directive.action;
                let origin = self.operation.key.request.directive.origin;
                if !super::transition::completes(action, origin) {
                    self.operation.key.request.directive.action = Action::Embark { parent: None };
                    self.operation.progress.execution.deadline = env::chrono::clock()
                        + crate::support::timing::BOOT_MS as u64 * 1_000_000;
                    self.cursor.reset();
                    return Ok(());
                }
                self.complete = true;
            }
            Ok(::schedule::Progress::Pending) => {}
            Err(fail) => {
                let job = &mut self.operation;
                let action = job.key.request.directive.action;
                let origin = job.key.request.directive.origin;
                if super::transition::cleanup_failure(action, job.progress.execution.task) {
                    programs::debug::put(&alloc::format!(
                        "system: ruin {} failed {:?}",
                        job.key.request.name,
                        fail
                    ));
                    return Err(crate::system::app::Fault::Shutdown);
                }
                job.progress.failure = Some(fail);
                if super::transition::rollback(action, origin, &job.progress.execution) {
                    job.key.request.directive.action = Action::Ruin;
                    job.key.request.directive.origin = Origin::Rollback;
                    job.progress.execution.deadline = env::chrono::clock()
                        + crate::support::timing::BOOT_MS as u64 * 1_000_000;
                    self.cursor.reset();
                } else {
                    self.complete = true;
                }
            }
        }
        Ok(())
    }
}
