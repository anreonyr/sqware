use env::TaskId;

use super::{DomainRange, Failure, Phase, ScopeId};

#[derive(Clone, Copy, Debug)]
pub struct Occupancy {
    pub scope: ScopeId,
    pub domain_range: DomainRange,
    pub controller: TaskId,
    pub phase: Phase,
    pub references: usize,
    pub failure: Option<Failure>,
}
