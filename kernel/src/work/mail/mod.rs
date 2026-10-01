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
/// 两边的段表各自走一遍、按页片段搬——`Segments` 每一片段给的是"当前 va 的物理地址 ＋
/// 本页内剩下的长度"，故一次搬两边**各自还剩**的那一段里较短的那一段。先 `whole` 验两边
/// （发送方可读、收方可写）；验不过一个字节都不动。
///
/// `ptr::copy`（memmove）而不是 `copy_nonoverlapping`：两边可能是同一批物理帧的两处映射
/// （借映那一轴），重叠时也没事。今天那两个方向各有一句自己的注释，见 `copy_in`／`copy_out`。
pub(crate) fn copy(src: &Space, src_va: usize, dst: &Space, dst_va: usize, len: usize) -> bool {
    if !whole(src, src_va, len, PteFlags::R) || !whole(dst, dst_va, len, PteFlags::W) {
        return false;
    }
    let mut from = src.segments(VirtAddr::from_raw(src_va), len);
    let mut into = dst.segments(VirtAddr::from_raw(dst_va), len);
    // 两侧游标：`(本片段当前物理地址, 本片段还剩几字节)`——**没搬完的片段不换下一片**
    // （旧正文每次都换，于是边界一错开就错到底）。
    let (mut spa, mut s_left) = (0usize, 0usize);
    let (mut dpa, mut d_left) = (0usize, 0usize);
    let mut off = 0;
    while off < len {
        if s_left == 0 {
            let Some((pa, _, l)) = from.next() else {
                return false;
            };
            spa = pa.as_usize();
            s_left = l;
        }
        if d_left == 0 {
            let Some((pa, _, l)) = into.next() else {
                return false;
            };
            dpa = pa.as_usize();
            d_left = l;
        }
        let n = s_left.min(d_left).min(len - off);
        // 空片段（构造上到不了：`whole` 已验两边整段覆盖）：不前进就当场收手，不绕圈。
        if n == 0 {
            return false;
        }
        // SAFETY: whole 已验两边权限与长度；两边各自的片段都不越出一页，而 `n` 不超过
        // 两边各自剩下的那一段 ⇒ 不会越过任何一边的映射。
        unsafe {
            core::ptr::copy(spa as *const u8, dpa as *mut u8, n);
        }
        spa += n;
        s_left -= n;
        dpa += n;
        d_left -= n;
        off += n;
    }
    true
}
