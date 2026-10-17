use alloc::{boxed::Box, vec::Vec};
use env::TaskId;

use super::{DomainRange, Failure, Phase, Platform, memory::Memory};

pub struct Scope {
    pub(super) domain_range: DomainRange,
    pub(super) controller: TaskId,
    pub(super) platform: Option<Box<dyn Platform>>,
    pub(super) phase: Phase,
    pub(super) memory: Vec<Memory>,
    pub(super) failure: Option<Failure>,
}

impl Scope {
    pub fn new(
        domain_range: DomainRange,
        controller: TaskId,
        platform: Option<Box<dyn Platform>>,
    ) -> Self {
        Self {
            domain_range,
            controller,
            platform,
            phase: Phase::Open,
            memory: Vec::new(),
            failure: None,
        }
    }
}
