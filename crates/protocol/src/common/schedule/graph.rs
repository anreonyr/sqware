use alloc::vec::Vec;
use super::{system::{System, IntoSystem}, plan::Plan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError { Duplicate, Unknown, Cycle, BorrowConflict, Room }
pub(crate) struct Node<L, E> { pub(crate) name: &'static str, pub(crate) phase: L, pub(crate) system: System<E> }
pub struct Schedule<L, E> { nodes: Vec<Node<L, E>>, edges: Vec<(&'static str, &'static str)> }
impl<L: Copy + Ord, E: 'static> Schedule<L, E> {
    pub fn new() -> Self { Self { nodes: Vec::new(), edges: Vec::new() } }
    pub fn add_system<M>(&mut self, name: &'static str, phase: L, f: impl IntoSystem<M, E>) -> Result<(), BuildError> {
        if self.nodes.iter().any(|n| n.name == name) { return Err(BuildError::Duplicate); }
        let system = f.into_system();
        for (i, a) in system.access.iter().enumerate() {
            if system.access[..i].iter().any(|b| a.id == b.id && (a.write || b.write)) { return Err(BuildError::BorrowConflict); }
        }
        self.nodes.try_reserve(1).map_err(|_| BuildError::Room)?;
        self.nodes.push(Node { name, phase, system });
        Ok(())
    }
    pub fn before(&mut self, first: &'static str, second: &'static str) -> Result<(), BuildError> {
        self.edges.try_reserve(1).map_err(|_| BuildError::Room)?;
        self.edges.push((first, second)); Ok(())
    }
    pub fn build(mut self) -> Result<Plan<E>, BuildError> {
        if self.edges.iter().any(|(a,b)| !self.nodes.iter().any(|n| n.name == *a) || !self.nodes.iter().any(|n| n.name == *b)) { return Err(BuildError::Unknown); }
        let mut steps = Vec::new();
        steps.try_reserve(self.nodes.len()).map_err(|_| BuildError::Room)?;
        while !self.nodes.is_empty() {
            let pick = self.nodes.iter().enumerate().filter(|(_, n)| {
                !self.nodes.iter().any(|other| other.phase < n.phase)
                && !self.edges.iter().any(|(a,b)| *b == n.name && self.nodes.iter().any(|other| other.name == *a))
            }).min_by_key(|(_,n)| n.name).map(|(i,_)| i).ok_or(BuildError::Cycle)?;
            steps.push(self.nodes.remove(pick).system);
        }
        Ok(Plan { steps })
    }
}
