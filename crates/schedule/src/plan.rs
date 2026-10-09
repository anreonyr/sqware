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
        self.at = 0;
        self.finishing = false;
        if let Some(nested) = &mut self.nested {
            nested.reset();
        }
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
            match step.execute(resources, cursor)? {
                Progress::Pending => return Ok(Progress::Pending),
                Progress::Done => {
                    cursor.at += 1;
                    if let Some(nested) = &mut cursor.nested {
                        nested.reset();
                    }
                    cursor.finishing = false;
                }
            }
        }
        Ok(Progress::Done)
    }
}

impl<E> Plan<E> {
    /// Bind declared resources before execution. Missing resources fail only when their step runs.
    pub fn prepare(&mut self, resources: &Resources<'_>) {
        for step in &mut self.steps {
            step.prepare(resources);
        }
    }
}
struct Mapped<E, F, M> {
    plan: Plan<E>,
    map: M,
    marker: core::marker::PhantomData<fn() -> F>,
}
impl<E, F, M: Fn(E) -> F> super::system::Runner<F> for Mapped<E, F, M> {
    fn prepare(&mut self, resources: &Resources<'_>) {
        self.plan.prepare(resources);
    }
    fn run(
        &mut self,
        resources: &Resources<'_>,
        cursor: &mut Cursor,
        _: &[usize; super::system::MAX_PARAMS],
    ) -> Result<Progress, RunError<F>> {
        let nested = cursor.nested.get_or_insert_with(Default::default);
        self.plan
            .advance(nested, resources)
            .map_err(|error| match error {
                RunError::Step(error) => RunError::Step((self.map)(error)),
                RunError::Resource(error) => RunError::Resource(error),
                RunError::UnknownPlan => RunError::UnknownPlan,
                RunError::Dispatch(error) => RunError::Dispatch(error),
            })
    }
}
impl<E: 'static> Plan<E> {
    pub fn map_error<F: 'static>(self, map: impl Fn(E) -> F + 'static) -> Plan<F> {
        Plan {
            steps: alloc::vec![System::new(
                [None; super::system::MAX_PARAMS],
                Mapped {
                    plan: self,
                    map,
                    marker: core::marker::PhantomData
                }
            )],
        }
    }
}
