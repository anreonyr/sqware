use super::{
    Cursor,
    resource::{Access, AccessError, Param, Resources},
};
use alloc::boxed::Box;
use core::marker::PhantomData;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress { Done, Pending }
#[derive(Debug, PartialEq, Eq)]
pub enum RunError<E> { Resource(AccessError), Step(E), UnknownPlan }
pub(crate) const MAX_PARAMS: usize = 10;
pub(crate) type Accesses = [Option<Access>; MAX_PARAMS];
pub(crate) trait Runner<E> {
    fn prepare(&mut self, _: &Resources<'_>) {}
    fn run(
        &mut self,
        resources: &Resources<'_>,
        cursor: &mut Cursor,
        slots: &[usize; MAX_PARAMS],
    ) -> Result<Progress, RunError<E>>;
}
pub struct System<E> {
    pub(crate) access: Accesses,
    slots: [usize; MAX_PARAMS],
    runner: Box<dyn Runner<E>>,
}
impl<E> System<E> {
    pub(crate) fn new(access: Accesses, runner: impl Runner<E> + 'static) -> Self {
        Self {
            access,
            slots: [usize::MAX; MAX_PARAMS],
            runner: Box::new(runner),
        }
    }
    pub(crate) fn prepare(&mut self, resources: &Resources<'_>) {
        for (at, access) in self.access.iter().enumerate() {
            if let Some(access) = access {
                self.slots[at] = resources
                    .index(access.id, self.slots[at])
                    .unwrap_or(usize::MAX);
            }
        }
        self.runner.prepare(resources);
    }
    pub(crate) fn execute(
        &mut self,
        resources: &Resources<'_>,
        cursor: &mut Cursor,
    ) -> Result<Progress, RunError<E>> {
        for (at, access) in self.access.iter().enumerate() {
            if let Some(access) = access {
                self.slots[at] = resources
                    .index(access.id, self.slots[at])
                    .map_err(RunError::Resource)?;
            }
        }
        self.runner.run(resources, cursor, &self.slots)
    }
}
pub trait IntoSystem<M, E> { fn into_system(self) -> System<E>; }
struct Function<F, M> {
    function: F,
    marker: PhantomData<fn() -> M>,
}
macro_rules! systems {
    ($($p:ident),*) => {
        impl<F, E: 'static, $($p: Param),*> Runner<E> for Function<F, fn($($p),*)>
        where F: 'static + FnMut($($p),*) -> Result<Progress, E>,
            for<'a> F: FnMut($($p::Item<'a>),*) -> Result<Progress, E>,
        {
            #[allow(non_snake_case)]
            fn run(&mut self, resources: &Resources<'_>, _: &mut Cursor, slots: &[usize; MAX_PARAMS]) -> Result<Progress, RunError<E>> {
                let mut at = 0;
                $(let $p = $p::get(resources, slots[at]).map_err(RunError::Resource)?; at += 1;)*
                let _ = at;
                (self.function)($($p),*).map_err(RunError::Step)
            }
        }
        impl<F, E: 'static, $($p: Param),*> IntoSystem<fn($($p),*), E> for F
        where F: 'static + FnMut($($p),*) -> Result<Progress, E>,
            for<'a> F: FnMut($($p::Item<'a>),*) -> Result<Progress, E>,
        {
            fn into_system(self) -> System<E> {
                let declared = [$($p::access()),*];
                System::new(core::array::from_fn(|at| declared.get(at).copied()), Function { function: self, marker: PhantomData::<fn() -> fn($($p),*)> })
            }
        }
    };
}
systems!(A);
systems!(A, B);
systems!(A, B, C);
systems!(A, B, C, D);
systems!(A, B, C, D, E0);
systems!(A, B, C, D, E0, F0);
systems!(A, B, C, D, E0, F0, G);
systems!(A, B, C, D, E0, F0, G, H);
systems!(A, B, C, D, E0, F0, G, H, I);
systems!(A, B, C, D, E0, F0, G, H, I, J);
impl<F, E: 'static> Runner<E> for Function<F, fn()>
where F: 'static + FnMut() -> Result<Progress, E> {
    fn run(
        &mut self,
        _: &Resources<'_>,
        _: &mut Cursor,
        _: &[usize; MAX_PARAMS],
    ) -> Result<Progress, RunError<E>> {
        (self.function)().map_err(RunError::Step)
    }
}
impl<F, E: 'static> IntoSystem<fn(), E> for F
where F: 'static + FnMut() -> Result<Progress, E> {
    fn into_system(self) -> System<E> {
        System::new(
            [None; MAX_PARAMS],
            Function {
                function: self,
                marker: PhantomData::<fn() -> fn()>,
            },
        )
    }
}
