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
    ) -> Result<&mut Self, BuildError> {
        if self.nodes.iter().any(|n| n.name == name) {
            return Err(BuildError::Duplicate);
        }
        let system = f.into_system();
        for (i, a) in system.access.iter().flatten().enumerate() {
            if system.access[..i]
                .iter()
                .flatten()
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
        Ok(self)
    }
    pub fn add_plan(
        &mut self,
        name: &'static str,
        phase: L,
        plan: Plan<E>,
    ) -> Result<&mut Self, BuildError> {
        self.node(name, phase, super::compose::nested(plan))
    }
    pub fn add_subplans<K: PartialEq + 'static, C: 'static, M, N>(
        &mut self,
        name: &'static str,
        phase: L,
        select: impl IntoSystem<M, E>,
        children: Vec<(K, Plan<C>)>,
        finish: impl IntoSystem<N, E>,
    ) -> Result<&mut Self, BuildError> {
        let select = select.into_system();
        let finish = finish.into_system();
        for system in [&select, &finish] {
            for (i, a) in system.access.iter().flatten().enumerate() {
                if system.access[..i]
                    .iter()
                    .flatten()
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
            super::compose::subplans(select, children, finish),
        )
    }
    fn node(
        &mut self,
        name: &'static str,
        phase: L,
        system: System<E>,
    ) -> Result<&mut Self, BuildError> {
        if self.nodes.iter().any(|n| n.name == name) {
            return Err(BuildError::Duplicate);
        }
        self.nodes.try_reserve(1).map_err(|_| BuildError::Room)?;
        self.nodes.push(Node {
            name,
            phase,
            system,
        });
        Ok(self)
    }
    pub fn before(
        &mut self,
        first: &'static str,
        second: &'static str,
    ) -> Result<&mut Self, BuildError> {
        self.edges.try_reserve(1).map_err(|_| BuildError::Room)?;
        self.edges.push((first, second));
        Ok(self)
    }
    /// Take the current configuration, leaving an empty schedule even if building fails.
    pub fn build(&mut self) -> Result<Plan<E>, BuildError> {
        let mut schedule = core::mem::replace(self, Self::new());
        if schedule.edges.iter().any(|(a, b)| {
            !schedule.nodes.iter().any(|n| n.name == *a)
                || !schedule.nodes.iter().any(|n| n.name == *b)
        }) {
            return Err(BuildError::Unknown);
        }
        let mut steps = Vec::new();
        steps
            .try_reserve(schedule.nodes.len())
            .map_err(|_| BuildError::Room)?;
        while !schedule.nodes.is_empty() {
            let pick = schedule
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    !schedule.nodes.iter().any(|other| other.phase < n.phase)
                        && !schedule.edges.iter().any(|(a, b)| {
                            *b == n.name && schedule.nodes.iter().any(|other| other.name == *a)
                        })
                })
                .next()
                .map(|(i, _)| i)
                .ok_or(BuildError::Cycle)?;
            steps.push(schedule.nodes.remove(pick).system);
        }
        Ok(Plan { steps })
    }
}
