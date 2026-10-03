use alloc::{boxed::Box, vec::Vec};
use super::resource::{Access, AccessError, Param, Resources};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress { Done, Pending }
#[derive(Debug, PartialEq, Eq)]
pub enum RunError<E> { Resource(AccessError), Step(E) }
pub struct System<E> {
    pub(crate) access: Vec<Access>,
    pub(crate) run: Box<dyn FnMut(&Resources<'_>) -> Result<Progress, RunError<E>>>,
}
pub trait IntoSystem<M, E> { fn into_system(self) -> System<E>; }

macro_rules! systems {
    ($($p:ident),*) => {
        impl<F, E: 'static, $($p: Param),*> IntoSystem<fn($($p),*), E> for F
        where F: 'static + FnMut($($p),*) -> Result<Progress, E>,
            for<'a> F: FnMut($($p::Item<'a>),*) -> Result<Progress, E>,
        {
            #[allow(non_snake_case)]
            fn into_system(mut self) -> System<E> {
                System {
                    access: alloc::vec![$($p::access()),*],
                    run: Box::new(move |resources| {
                        $(let $p = $p::get(resources).map_err(RunError::Resource)?;)*
                        self($($p),*).map_err(RunError::Step)
                    }),
                }
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
