use std::{cell::RefCell, rc::Rc};

use crate::{
    accessor::Accessor,
    retention::{DeviceRange, Failure, Platform},
};

pub struct Device {
    pub range: DeviceRange,
    pub accessor: Rc<RefCell<Accessor>>,
}

impl Platform for Device {
    fn range(&self) -> DeviceRange {
        self.range
    }

    fn synchronize(&mut self) -> Result<(), Failure> {
        let mut accessor = self.accessor.borrow_mut();
        accessor.synchronized += 1;
        accessor.stop()
    }

    fn isolate(&mut self) -> Result<(), Failure> {
        let mut accessor = self.accessor.borrow_mut();
        accessor.isolated += 1;
        accessor.stop()
    }
}
