use core::cell::Cell;

use crate::reader::Reader;

pub struct Sparse {
    limit: u32,
    present: Vec<(u16, u16, u16, u32)>,
    reads: Cell<usize>,
    failed: Option<(u16, u16)>,
}

impl Sparse {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            present: Vec::new(),
            reads: Cell::new(0),
            failed: None,
        }
    }

    pub fn present(&mut self, requester: u16, vendor: u16, device: u16, class: u32) {
        self.present
            .push((requester, vendor, device, class & 0x00FF_FFFF));
    }

    pub fn reads(&self) -> usize {
        self.reads.get()
    }

    pub fn fail(&mut self, requester: u16, register: u16) {
        self.failed = Some((requester, register));
    }
}

impl Reader for Sparse {
    fn limit(&self) -> u32 {
        self.limit
    }

    fn read8(&self, requester: u16, register: u16) -> Option<u8> {
        if u32::from(requester) >= self.limit {
            return None;
        }
        self.reads.set(self.reads.get() + 1);
        if self.failed == Some((requester, register)) {
            return None;
        }
        let Some(entry) = self.present.iter().find(|entry| entry.0 == requester) else {
            return Some(0xFF);
        };
        Some(match register {
            0 => entry.1 as u8,
            1 => (entry.1 >> 8) as u8,
            2 => entry.2 as u8,
            3 => (entry.2 >> 8) as u8,
            9 => entry.3 as u8,
            10 => (entry.3 >> 8) as u8,
            11 => (entry.3 >> 16) as u8,
            _ => 0,
        })
    }
}
