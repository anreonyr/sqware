use alloc::sync::Arc;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::{PhysAddr, VirtAddr};
use crate::core::memory::nest::table::{PteFlags, TableNode};
use crate::core::work::unit::space::Space;

#[derive(Clone)]
pub struct StackReader {
    space: Option<Arc<Space>>,
    root: PhysAddr,
    page: Option<(usize, PhysAddr)>,
}

impl StackReader {
    pub fn new(root_ppn: usize) -> StackReader {
        StackReader {
            space: None,
            root: PhysAddr::from_raw(root_ppn << 12),
            page: None,
        }
    }

    pub fn user(space: Arc<Space>) -> Self {
        Self {
            space: Some(space),
            root: PhysAddr::from_raw(0),
            page: None,
        }
    }

    fn leaf(&mut self, page: usize) -> Option<PhysAddr> {
        if let Some((cached, base)) = self.page
            && cached == page
        {
            return Some(base);
        }
        let edge = crate::machine::board::dram_edge().unwrap_or(0x9000_0000);
        let (base, flags) = TableNode::walk_raw(
            self.root,
            VirtAddr::from_raw(page),
            |pa| (0x8000_0000..edge).contains(&pa.as_usize()),
        )?;
        if !flags.contains(PteFlags::R) {
            return None;
        }
        self.page = Some((page, base));
        Some(base)
    }

    pub fn word(&mut self, addr: usize) -> Option<usize> {
        if let Some(space) = &self.space {
            let mut bytes = [0u8; size_of::<usize>()];
            return space
                .memory
                .copy_in(&mut bytes, addr)
                .then(|| usize::from_le_bytes(bytes));
        }
        if (addr & (PAGE_SIZE - 1)) + size_of::<usize>() > PAGE_SIZE {
            return None;
        }
        let page = addr & !(PAGE_SIZE - 1);
        let base = self.leaf(page)?;
        Some(unsafe {
            (base.as_usize() as *const u8)
                .add(addr - page)
                .cast::<usize>()
                .read_unaligned()
        })
    }

    pub fn pair(&mut self, frame: usize) -> Option<(usize, usize)> {
        Some((
            self.word(frame.checked_sub(16)?)?,
            self.word(frame.checked_sub(8)?)?,
        ))
    }
}
