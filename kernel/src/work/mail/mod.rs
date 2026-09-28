pub mod hole;
pub mod nole;
pub mod pole;
pub mod tole;

pub(crate) use hole::HoleMeta;
pub(crate) use pole::PoleMeta;
pub(crate) use tole::ToleMeta;

use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::Space;

fn whole(space: &Space, va: usize, len: usize, need: PteFlags) -> bool {
    let mut off = 0;
    for (_, flags, l) in space.segments(VirtAddr::from_raw(va), len) {
        if !flags.intersects(need) || off + l > len {
            return false;
        }
        off += l;
    }
    off == len
}

pub(crate) fn copy_in(space: &Space, dst: &mut [u8], va: usize) -> bool {
    if !whole(space, va, dst.len(), PteFlags::R) {
        return false;
    }
    let mut off = 0;
    for (pa, _, l) in space.segments(VirtAddr::from_raw(va), dst.len()) {
        // SAFETY: whole 已验本段权限与长度；两遍之间只可能变窄
        unsafe {
            core::ptr::copy_nonoverlapping(
                pa.as_usize() as *const u8,
                dst.as_mut_ptr().add(off),
                l,
            );
        }
        off += l;
    }
    true
}

pub(crate) fn copy_out(space: &Space, src: &[u8], va: usize) -> bool {
    if !whole(space, va, src.len(), PteFlags::W) {
        return false;
    }
    let mut off = 0;
    for (pa, _, l) in space.segments(VirtAddr::from_raw(va), src.len()) {
        // SAFETY: 同上
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr().add(off), pa.as_usize() as *mut u8, l);
        }
        off += l;
    }
    true
}