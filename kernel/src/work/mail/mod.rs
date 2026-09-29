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

pub(crate) fn whole(space: &Space, va: usize, len: usize, need: PteFlags) -> bool {
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

/// **一处复制**：发送方那段 → 收方那段（跨空间，一次）。
///
/// 两边的段表各自走一遍、按页片段锁步搬——`Segments` 每一片段给的是"当前 va 的物理地址 ＋
/// 本页内剩下的长度"，故取两边较小者即可（两个片段各落在一枚帧内）。先 `whole` 验两边
/// （发送方可读、收方可写）；验不过一个字节都不动。
///
/// `ptr::copy`（memmove）而不是 `copy_nonoverlapping`：两边可能是同一批物理帧的两处映射
/// （借映那一轴），重叠时也没事。今天那两个方向各有一句自己的注释，见 `copy_in`／`copy_out`。
pub(crate) fn copy(src: &Space, src_va: usize, dst: &Space, dst_va: usize, len: usize) -> bool {
    if !whole(src, src_va, len, PteFlags::R) || !whole(dst, dst_va, len, PteFlags::W) {
        return false;
    }
    let mut off = 0;
    let mut from = src.segments(VirtAddr::from_raw(src_va), len);
    let mut into = dst.segments(VirtAddr::from_raw(dst_va), len);
    while off < len {
        let (Some((spa, _, sl)), Some((dpa, _, dl))) = (from.next(), into.next()) else {
            return false;
        };
        let n = sl.min(dl).min(len - off);
        // SAFETY: whole 已验两边权限与长度；两个片段各在一枚帧内（Segments 按页切），
        // 故 n 字节不会越过任何一边的映射。
        unsafe {
            core::ptr::copy(spa.as_usize() as *const u8, dpa.as_usize() as *mut u8, n);
        }
        off += n;
    }
    true
}
