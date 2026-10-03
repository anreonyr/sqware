use crate::memory::PAGE_SIZE;
use crate::memory::manager::{addr::VirtAddr, entry::PteFlags, mode};

use super::{Pending, Space, inner::SpaceInner};

impl Space {
    /// Caller-visible addresses retain their raw value and stay in the lower half.
    pub(crate) fn user_range(va: usize, len: usize) -> bool {
        va < mode::upper().as_usize()
            && va
                .checked_add(len)
                .is_some_and(|end| end <= mode::upper().as_usize())
    }

    #[cfg(debug_assertions)]
    pub(crate) fn validate_read(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, PteFlags::R)
    }

    pub(crate) fn validate_write(&self, va: usize, len: usize) -> bool {
        self.validate_access(va, len, PteFlags::W)
    }

    pub(crate) fn validate_access(&self, va: usize, len: usize, need: PteFlags) -> bool {
        self.with(|inner| self.access(inner, va, len, need))
    }

    fn access(&self, inner: &SpaceInner, va: usize, len: usize, need: PteFlags) -> bool {
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
        let mut at = va;
        let end = va + len;
        while at < end {
            let addr = VirtAddr::wrap(at);
            let Some(map) = inner.resolve_ref(addr) else {
                return false;
            };
            if map.pending == Some(Pending::Guard) {
                return false;
            }
            let Some((_, flags)) = inner.translate(addr) else {
                return false;
            };
            if !flags.contains(need) || flags.contains(PteFlags::G) {
                return false;
            }
            at += (PAGE_SIZE - addr.offset()).min(end - at);
        }
        true
    }

    pub(crate) fn instruction_byte(&self, va: usize) -> Option<u8> {
        let mut byte = [0u8];
        self.copy_from(&mut byte, va, PteFlags::X)
            .then_some(byte[0])
    }

    pub(crate) fn copy_in(&self, dst: &mut [u8], va: usize) -> bool {
        self.copy_from(dst, va, PteFlags::R)
    }

    fn copy_from(&self, dst: &mut [u8], va: usize, need: PteFlags) -> bool {
        self.with(|inner| {
            if !self.access(inner, va, dst.len(), need) {
                return false;
            }
            let mut off = 0;
            while off < dst.len() {
                let addr = VirtAddr::wrap(va + off);
                let (pa, _) = inner.translate(addr).expect("validated mapping");
                let n = (PAGE_SIZE - addr.offset()).min(dst.len() - off);
                // SAFETY: validation and copying hold the mapping lock together.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        pa.as_usize() as *const u8,
                        dst.as_mut_ptr().add(off),
                        n,
                    );
                }
                off += n;
            }
            true
        })
    }

    pub(crate) fn copy_out(&self, src: &[u8], va: usize) -> bool {
        self.with(|inner| {
            if !self.access(inner, va, src.len(), PteFlags::W) {
                return false;
            }
            let mut off = 0;
            while off < src.len() {
                let addr = VirtAddr::wrap(va + off);
                let (pa, _) = inner.translate(addr).expect("validated mapping");
                let n = (PAGE_SIZE - addr.offset()).min(src.len() - off);
                // SAFETY: validation and copying hold the mapping lock together.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        src.as_ptr().add(off),
                        pa.as_usize() as *mut u8,
                        n,
                    );
                }
                off += n;
            }
            true
        })
    }
}
