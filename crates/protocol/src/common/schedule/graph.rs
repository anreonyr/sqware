use super::{
    plan::Plan,
    system::{IntoSystem, System},
};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    Duplicate,
    Unknown,
    Cycle,
    BorrowConflict,
    Room,
}
pub(crate) struct Node<L, E> {
    pub(crate) name: &'static str,
    pub(crate) phase: L,
    pub(crate) system: System<E>,
}
pub struct Schedule<L, E> {
    nodes: Vec<Node<L, E>>,
    edges: Vec<(&'static str, &'static str)>,
}
impl<L: Copy + Ord, E: 'static> Schedule<L, E> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
    pub fn add_system<M>(
        &mut self,
        name: &'static str,
        phase: L,
        f: impl IntoSystem<M, E>,
    ) -> Result<(), BuildError> {
        if self.nodes.iter().any(|n| n.name == name) {
            return Err(BuildError::Duplicate);
        }
        let system = f.into_system();
        for (i, a) in system.access.iter().enumerate() {
            if system.access[..i]
                .iter()
                .any(|b| a.id == b.id && (a.write || b.write))
            {
                return Err(BuildError::BorrowConflict);
            }
        }
        self.nodes.try_reserve(1).map_err(|_| BuildError::Room)?;
        self.nodes.push(Node {
            name,
            phase,
            system,
        });
        Ok(())
    }
    pub fn add_plan(
        &mut self,
        name: &'static str,
        phase: L,
        mut plan: Plan<E>,
    ) -> Result<(), BuildError> {
        self.node(
            name,
            phase,
            System {
                access: Vec::new(),
                run: alloc::boxed::Box::new(move |resources, cursor| {
                    let nested = cursor.nested.get_or_insert_with(Default::default);
                    let result = plan.advance(nested, resources)?;
                    if result == super::Progress::Done {
                        cursor.nested = None;
                    }
                    Ok(result)
                }),
            },
        )
    }
    pub fn add_subplans<K: PartialEq + 'static, C: 'static, M, N>(
        &mut self,
        name: &'static str,
        phase: L,
        select: impl IntoSystem<M, E>,
        mut children: Vec<(K, Plan<C>)>,
        finish: impl IntoSystem<N, E>,
    ) -> Result<(), BuildError> {
        let mut select = select.into_system();
        let mut finish = finish.into_system();
        for system in [&select, &finish] {
            for (i, a) in system.access.iter().enumerate() {
                if system.access[..i]
                    .iter()
                    .any(|b| a.id == b.id && (a.write || b.write))
                {
                    return Err(BuildError::BorrowConflict);
                }
            }
        }
        for (i, (key, _)) in children.iter().enumerate() {
            if children[..i].iter().any(|(other, _)| other == key) {
                return Err(BuildError::Duplicate);
            }
        }
        self.node(
            name,
            phase,
            System {
                access: Vec::new(),
                run: alloc::boxed::Box::new(move |resources, cursor| {
                    loop {
                        if cursor.finishing {
                            if (finish.run)(resources, cursor)? == super::Progress::Pending {
                                return Ok(super::Progress::Pending);
                            }
                            cursor.finishing = false;
                        }
                        if resources
                            .read::<super::Dispatch<K, C>>()
                            .map_err(super::RunError::Resource)?
                            .budget
                            == 0
                        {
                            break;
                        }
                        if (select.run)(resources, cursor)? == super::Progress::Pending {
                            return Ok(super::Progress::Pending);
                        }
                        let invocation = resources
                            .write::<super::Dispatch<K, C>>()
                            .map_err(super::RunError::Resource)?
                            .current
                            .take();
                        let Some(mut invocation) = invocation else {
                            break;
                        };
                        let plan = children
                            .iter_mut()
                            .find(|(key, _)| *key == invocation.key)
                            .map(|(_, plan)| plan)
                            .ok_or(super::RunError::UnknownPlan)?;
                        let result = plan.advance(&mut invocation.cursor, resources);
                        {
                            let mut dispatch = resources
                                .write::<super::Dispatch<K, C>>()
                                .map_err(super::RunError::Resource)?;
                            dispatch.current = Some(invocation);
                            dispatch.result = Some(result);
                            dispatch.budget -= 1;
                        }
                        cursor.finishing = true;
                    }
                    Ok(super::Progress::Done)
                }),
            },
        )
    }
    fn node(&mut self, name: &'static str, phase: L, system: System<E>) -> Result<(), BuildError> {
        if self.nodes.iter().any(|n| n.name == name) {
            return Err(BuildError::Duplicate);
        }
        self.nodes.try_reserve(1).map_err(|_| BuildError::Room)?;
        self.nodes.push(Node {
            name,
            phase,
            system,
        });
        Ok(())
    }
    pub fn before(&mut self, first: &'static str, second: &'static str) -> Result<(), BuildError> {
        self.edges.try_reserve(1).map_err(|_| BuildError::Room)?;
        self.edges.push((first, second));
        Ok(())
    }
    pub fn build(mut self) -> Result<Plan<E>, BuildError> {
        if self.edges.iter().any(|(a, b)| {
            !self.nodes.iter().any(|n| n.name == *a) || !self.nodes.iter().any(|n| n.name == *b)
        }) {
            return Err(BuildError::Unknown);
        }
        let mut steps = Vec::new();
        steps
            .try_reserve(self.nodes.len())
            .map_err(|_| BuildError::Room)?;
        while !self.nodes.is_empty() {
            let pick = self
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    !self.nodes.iter().any(|other| other.phase < n.phase)
                        && !self.edges.iter().any(|(a, b)| {
                            *b == n.name && self.nodes.iter().any(|other| other.name == *a)
                        })
                })
                .min_by_key(|(_, n)| n.name)
                .map(|(i, _)| i)
                .ok_or(BuildError::Cycle)?;
            steps.push(self.nodes.remove(pick).system);
        }
        Ok(Plan { steps })
    }
}
