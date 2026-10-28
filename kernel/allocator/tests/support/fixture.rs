use core::alloc::{Allocator, Layout};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use kernel_allocator::PAGE_SIZE;
use kernel_allocator::bump::BumpAllocator;
use kernel_allocator::frame::Effect;
use kernel_allocator::{InitError, MachineFacts, Region};

use super::lock::TestLock;

pub(crate) struct Fixture<'a> {
    pub(crate) bump: &'a BumpAllocator<TestLock>,
    pub(crate) reserved: Region,
    pub(crate) install_addr: Arc<AtomicUsize>,
}

unsafe impl Effect for Fixture<'_> {
    fn facts(&self) -> MachineFacts {
        MachineFacts {
            free: Region::new(0, 0),
            reserved: [Some(self.reserved), None],
        }
    }

    fn bump_boundary(&self) -> usize {
        self.bump.boundary()
    }

    fn bump_frontier(&self) -> usize {
        self.bump.frontier()
    }

    fn install_frame_kinds(&self, _frames: usize) -> Result<(), InitError> {
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        let page = self.bump.allocate(layout).expect("classified metadata");
        self.install_addr
            .store(page.as_ptr() as *mut u8 as usize, Ordering::Relaxed);
        Ok(())
    }

    fn record_frame_take(&self, _index: usize, _power: usize) {}
    fn record_frame_give(&self, _index: usize) {}
    fn assert_allocation_allowed(&self) {}
    fn report(&self, args: core::fmt::Arguments<'_>) {
        eprintln!("{args}");
    }
}
