use env::{TaskId, TeamId};

use super::ExecutionFailure;

pub trait ExecutionEffects {
    fn team(&mut self, task: TaskId) -> Result<Option<TeamId>, ExecutionFailure>;
    fn embark(&mut self, task: TaskId) -> Result<(), ExecutionFailure>;
    fn exited(&mut self, task: TaskId) -> Result<bool, ExecutionFailure>;
    fn stop(&mut self, team: TeamId) -> Result<(), ExecutionFailure>;
    fn oust(&mut self, team: TeamId) -> Result<bool, ExecutionFailure>;
}
