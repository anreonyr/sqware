use alloc::vec::Vec;
use super::{resource::Resources, system::{System, Progress, RunError}};

/// Execution position for one plan. Reset before switching plans.
#[derive(Default)]
pub struct Cursor { at: usize }
pub struct Plan<E> { pub(crate) steps: Vec<System<E>> }
impl Cursor { pub fn reset(&mut self) { self.at = 0; } }
impl<E> Plan<E> {
    /// Completed steps are skipped; Pending and errors leave the current step in place.
    pub fn advance(&mut self, cursor: &mut Cursor, resources: &Resources<'_>) -> Result<Progress, RunError<E>> {
        while let Some(step) = self.steps.get_mut(cursor.at) {
            match (step.run)(resources)? {
                Progress::Pending => return Ok(Progress::Pending),
                Progress::Done => cursor.at += 1,
            }
        }
        Ok(Progress::Done)
    }
}
