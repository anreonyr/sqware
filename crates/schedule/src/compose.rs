use super::{Plan, system::System};
use alloc::vec::Vec;

pub(crate) fn nested<E: 'static>(plan: Plan<E>) -> System<E> {
    System::new([None; super::system::MAX_PARAMS], Nested { plan })
}
pub(crate) fn subplans<K: PartialEq + 'static, C: 'static, E: 'static>(
    select: System<E>,
    children: Vec<(K, Plan<C>)>,
    finish: System<E>,
) -> System<E> {
    System::new(
        [None; super::system::MAX_PARAMS],
        Subplans {
            select,
            children,
            finish,
            slot: usize::MAX,
        },
    )
}

struct Nested<E> {
    plan: Plan<E>,
}
impl<E> super::system::Runner<E> for Nested<E> {
    fn prepare(&mut self, resources: &super::Resources<'_>) {
        self.plan.prepare(resources);
    }
    fn run(
        &mut self,
        resources: &super::Resources<'_>,
        cursor: &mut super::Cursor,
        _: &[usize; super::system::MAX_PARAMS],
    ) -> Result<super::Progress, super::RunError<E>> {
        let nested = cursor.nested.get_or_insert_with(Default::default);
        self.plan.advance(nested, resources)
    }
}
struct Subplans<K, C, E> {
    select: System<E>,
    children: Vec<(K, Plan<C>)>,
    finish: System<E>,
    slot: usize,
}
impl<K: PartialEq + 'static, C: 'static, E> super::system::Runner<E> for Subplans<K, C, E> {
    fn prepare(&mut self, resources: &super::Resources<'_>) {
        self.slot = resources
            .index(core::any::TypeId::of::<super::Dispatch<K, C>>(), self.slot)
            .unwrap_or(usize::MAX);
        self.select.prepare(resources);
        self.finish.prepare(resources);
        for (_, plan) in &mut self.children {
            plan.prepare(resources);
        }
    }
    fn run(
        &mut self,
        resources: &super::Resources<'_>,
        cursor: &mut super::Cursor,
        _: &[usize; super::system::MAX_PARAMS],
    ) -> Result<super::Progress, super::RunError<E>> {
        self.slot = resources
            .index(core::any::TypeId::of::<super::Dispatch<K, C>>(), self.slot)
            .map_err(super::RunError::Resource)?;
        loop {
            if cursor.finishing {
                if self.finish.execute(resources, cursor)? == super::Progress::Pending {
                    return Ok(super::Progress::Pending);
                }
                if !resources
                    .read_at::<super::Dispatch<K, C>>(self.slot)
                    .map_err(super::RunError::Resource)?
                    .is_idle()
                {
                    return Err(super::RunError::Dispatch(super::DispatchError::Busy));
                }
                cursor.finishing = false;
            }
            if resources
                .read_at::<super::Dispatch<K, C>>(self.slot)
                .map_err(super::RunError::Resource)?
                .remaining()
                == 0
            {
                break;
            }
            let selected = resources
                .read_at::<super::Dispatch<K, C>>(self.slot)
                .map_err(super::RunError::Resource)?
                .has_selected();
            if !selected && self.select.execute(resources, cursor)? == super::Progress::Pending {
                return Ok(super::Progress::Pending);
            }
            let invocation = resources
                .write_at::<super::Dispatch<K, C>>(self.slot)
                .map_err(super::RunError::Resource)?
                .take_selected();
            let Some(mut invocation) = invocation else {
                break;
            };
            let plan = self
                .children
                .iter_mut()
                .find(|(key, _)| *key == invocation.key)
                .map(|(_, plan)| plan);
            let Some(plan) = plan else {
                resources
                    .write_at::<super::Dispatch<K, C>>(self.slot)
                    .map_err(super::RunError::Resource)?
                    .restore_selected(invocation)
                    .map_err(super::RunError::Dispatch)?;
                return Err(super::RunError::UnknownPlan);
            };
            let result = plan.advance(&mut invocation.cursor, resources);
            {
                let mut dispatch = resources
                    .write_at::<super::Dispatch<K, C>>(self.slot)
                    .map_err(super::RunError::Resource)?;
                dispatch
                    .complete(super::Completion { invocation, result })
                    .map_err(super::RunError::Dispatch)?;
            }
            cursor.finishing = true;
        }
        Ok(super::Progress::Done)
    }
}
