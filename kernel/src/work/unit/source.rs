use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr;

use super::space::Space;

pub(crate) const HEAD: usize = PAGE_SIZE;

#[derive(Clone, Copy)]
pub(crate) enum Source<'a> {
    Slice(&'a [u8]),
    Space {
        space: &'a Space,
        va: VirtAddr,
        len: usize,
    },
}

impl Source<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            Source::Slice(bytes) => bytes.len(),
            Source::Space { len, .. } => *len,
        }
    }

    pub(crate) fn read(&self, off: usize, dst: &mut [u8]) -> bool {
        let Some(end) = off.checked_add(dst.len()) else {
            return false;
        };
        if end > self.len() {
            return false;
        }
        match self {
            Source::Slice(bytes) => {
                dst.copy_from_slice(&bytes[off..end]);
                true
            }
            Source::Space { space, va, .. } => {
                let Some(raw) = va.as_usize().checked_add(off) else {
                    return false;
                };
                let at = VirtAddr::from_raw(raw);
                if at.as_usize().checked_add(dst.len()).is_none() {
                    return false;
                }
                let mut done = 0usize;
                for (pa, _flags, chunk) in space.segments(at, dst.len()) {
                    // SAFETY: pa 为恒等映射的物理地址
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            pa.as_usize() as *const u8,
                            dst.as_mut_ptr().add(done),
                            chunk,
                        );
                    }
                    done += chunk;
                }
                done == dst.len()
            }
        }
    }
}
