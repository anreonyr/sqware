use super::{
    super::super::retention_core::{DomainRange, Failure, Platform},
    accessor::Accessor,
};
use alloc::rc::Rc;

pub(super) struct Device {
    pub domain_range: DomainRange,
    pub accessor: Rc<Accessor>,
}

impl Platform for Device {
    fn domain_range(&self) -> DomainRange {
        self.domain_range
    }
    fn synchronize(&mut self) -> Result<(), Failure> {
        self.isolate()
    }
    fn isolate(&mut self) -> Result<(), Failure> {
        if self.accessor.timeout.get() {
            return Err(Failure::Pending);
        }
        self.accessor.active.set(false);
        Ok(())
    }
}
