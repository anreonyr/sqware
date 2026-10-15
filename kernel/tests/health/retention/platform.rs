use super::{
    super::super::retention_core::{DeviceRange, Failure, Platform},
    accessor::Accessor,
};
use alloc::rc::Rc;

pub(super) struct Device {
    pub range: DeviceRange,
    pub accessor: Rc<Accessor>,
}

impl Platform for Device {
    fn range(&self) -> DeviceRange {
        self.range
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
