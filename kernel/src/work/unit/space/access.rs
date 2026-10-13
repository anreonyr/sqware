use crate::memory::manager::{addr::VirtAddr, entry::PteFlags};

use super::Space;

impl Space {
    #[cfg(debug_assertions)]
    pub(crate) fn validate_read(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, PteFlags::R)
    }

    pub(crate) fn validate_write(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, PteFlags::W)
    }

    pub(crate) fn validate_access(&self, va: usize, len: usize, need: PteFlags) -> bool {
        if !Self::user_range(va, len) {
            return false;
        }
        let need = need
            | PteFlags::V
            | if self.kind().is_supervisor() {
                PteFlags::empty()
            } else {
                PteFlags::U
            };
        self.tree.check_access(VirtAddr::wrap(va), len, need)
    }

    pub(crate) fn instruction_byte(&self, va: usize) -> Option<u8> {
        let mut byte = [0];
        self.copy_from(&mut byte, va, PteFlags::X)
            .then_some(byte[0])
    }

    pub(crate) fn copy_in(&self, dst: &mut [u8], va: usize) -> bool {
        self.copy_from(dst, va, PteFlags::R)
    }

    fn copy_from(&self, dst: &mut [u8], va: usize, need: PteFlags) -> bool {
        if !Self::user_range(va, dst.len()) {
            return false;
        }
        let need = need
            | PteFlags::V
            | if self.kind().is_supervisor() {
                PteFlags::empty()
            } else {
                PteFlags::U
            };
        self.tree.copy_from(VirtAddr::wrap(va), dst, need)
    }

    pub(crate) fn copy_out(&self, src: &[u8], va: usize) -> bool {
        if !Self::user_range(va, src.len()) {
            return false;
        }
        let need = PteFlags::W
            | PteFlags::V
            | if self.kind().is_supervisor() {
                PteFlags::empty()
            } else {
                PteFlags::U
            };
        self.tree.copy_to(VirtAddr::wrap(va), src, need)
    }
}
