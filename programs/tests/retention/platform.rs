use std::{cell::RefCell, rc::Rc};

use crate::{
    accessor::Accessor,
    retention::{DomainRange, Failure, Platform},
};

pub struct Device {
    pub domain_range: DomainRange,
    pub accessor: Rc<RefCell<Accessor>>,
}

impl Platform for Device {
    fn domain_range(&self) -> DomainRange {
        self.domain_range
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
