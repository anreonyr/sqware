use super::{BuildError, IntoSystem, Plan, Schedule};
use alloc::vec::Vec;

/// A schedule whose steps execute in declaration order.
pub struct Sequence<E> {
    schedule: Schedule<usize, E>,
    next: usize,
}
impl<E: 'static> Schedule<(), E> {
    pub fn sequence() -> Sequence<E> {
        Sequence { schedule: Schedule::new(), next: 0 }
    }
}
impl<E: 'static> Sequence<E> {
    pub fn system<M>(&mut self, name: &'static str, f: impl IntoSystem<M, E>) -> Result<&mut Self, BuildError> {
        self.schedule.add_system(name, self.next, f)?;
        self.next += 1;
        Ok(self)
    }
    pub fn plan(&mut self, name: &'static str, plan: Plan<E>) -> Result<&mut Self, BuildError> {
        self.schedule.add_plan(name, self.next, plan)?;
        self.next += 1;
        Ok(self)
    }
    pub fn subplans<K: PartialEq + 'static, C: 'static, M, N>(
        &mut self,
        name: &'static str,
        select: impl IntoSystem<M, E>,
        children: Vec<(K, Plan<C>)>,
        finish: impl IntoSystem<N, E>,
    ) -> Result<&mut Self, BuildError> {
        self.schedule.add_subplans(name, self.next, select, children, finish)?;
        self.next += 1;
        Ok(self)
    }
    /// Take the current configuration and reset declaration order, even on failure.
    pub fn build(&mut self) -> Result<Plan<E>, BuildError> {
        self.next = 0;
        self.schedule.build()
    }
}
