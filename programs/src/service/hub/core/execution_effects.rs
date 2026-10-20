use env::{PieToken, TaskId, TeamId};

use super::ExecutionFailure;

pub trait ExecutionEffects {
    fn team(&mut self, task: TaskId) -> Result<Option<TeamId>, ExecutionFailure>;
    fn create(&mut self) -> Result<PieToken, ExecutionFailure>;
    fn populate(&mut self, startup: PieToken) -> Result<(), ExecutionFailure>;
    fn deliver(&mut self, startup: PieToken, task: TaskId) -> Result<(), ExecutionFailure>;
    fn restrict(&mut self, startup: PieToken) -> Result<(), ExecutionFailure>;
    fn embark(&mut self, task: TaskId) -> Result<(), ExecutionFailure>;
    fn exited(&mut self, task: TaskId) -> Result<bool, ExecutionFailure>;
    fn stop(&mut self, team: TeamId) -> Result<(), ExecutionFailure>;
    fn oust(&mut self, team: TeamId) -> Result<bool, ExecutionFailure>;
    fn release(&mut self, startup: PieToken) -> Result<(), ExecutionFailure>;
}
