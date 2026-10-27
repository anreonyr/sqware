use alloc::{boxed::Box, vec::Vec};
use env::TaskId;

use super::{DeviceRange, Failure, Phase, Platform, memory::Memory};

pub struct Scope {
    pub(super) range: DeviceRange,
    pub(super) controller: TaskId,
    pub(super) platform: Option<Box<dyn Platform>>,
    pub(super) phase: Phase,
    pub(super) memory: Vec<Memory>,
    pub(super) failure: Option<Failure>,
}

impl Scope {
    pub fn new(
        range: DeviceRange,
        controller: TaskId,
        platform: Option<Box<dyn Platform>>,
    ) -> Self {
        Self {
            range,
            controller,
            platform,
            phase: Phase::Open,
            memory: Vec::new(),
            failure: None,
        }
    }
}
