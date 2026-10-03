use super::{
    resource::Resources,
    system::{Progress, RunError, System},
};
use alloc::vec::Vec;

/// Execution position for one plan. Reset before switching plans.
#[derive(Default)]
pub struct Cursor {
    pub(crate) at: usize,
    pub(crate) nested: Option<alloc::boxed::Box<Cursor>>,
    pub(crate) finishing: bool,
}
pub struct Plan<E> {
    pub(crate) steps: Vec<System<E>>,
}
impl Cursor {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
impl<E> Plan<E> {
    /// Completed steps are skipped; Pending and errors leave the current step in place.
    pub fn advance(
        &mut self,
        cursor: &mut Cursor,
        resources: &Resources<'_>,
    ) -> Result<Progress, RunError<E>> {
        while let Some(step) = self.steps.get_mut(cursor.at) {
            match (step.run)(resources, cursor)? {
                Progress::Pending => return Ok(Progress::Pending),
                Progress::Done => {
                    cursor.at += 1;
                    cursor.nested = None;
                    cursor.finishing = false;
                }
            }
        }
        Ok(Progress::Done)
    }
}

impl<E: 'static> Plan<E> {
    pub fn map_error<F: 'static>(self, map: impl Fn(E) -> F + Clone + 'static) -> Plan<F> {
        Plan {
            steps: self
                .steps
                .into_iter()
                .map(|mut step| {
                    let map = map.clone();
                    System {
                        access: step.access,
                        run: alloc::boxed::Box::new(move |resources, cursor| {
                            (step.run)(resources, cursor).map_err(|error| match error {
                                RunError::Step(error) => RunError::Step(map(error)),
                                RunError::Resource(error) => RunError::Resource(error),
                                RunError::UnknownPlan => RunError::UnknownPlan,
                            })
                        }),
                    }
                })
                .collect(),
        }
    }
}
