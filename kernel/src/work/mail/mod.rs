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
///
/// # 照实记（**两面片段边界对不齐**——这一格是量出来的，旧正文在这里错了两处）
///
/// `Segments` 的片段边界由**每一侧自己的 `va` 在一页里的偏移**定：源与收两处 `va` 一般落在
/// 页内不同位置（`0x3fb10` 与 `0x43ff8` 那种），故两边的片段**长短不同、边界也对不齐**。
/// 旧正文**每个片段只搬 `min(sl, dl)`、却把两个迭代器各自推进一整段**（`next()` 一步就把
/// 本段走完）——两边于是从第二个片段起就**错位**，两种下场都量到过：
///
///   · **收方那一片更短** ⇒ 搬完 `dl` 之后 `off < len`，而源那一边已经把自己的段交完了
///     ⇒ 这一手答 `false` ⇒ 内核把这条报当**"发送方那段没了"**就地扔掉、答 `Gone`。
///     **实测**（debug 档 `product` 景，逐跑复现同一行）：
///     `mail: hand gone hole#83 from=14 owner=15 va=0x3fb10 len=10 dst=0x43ff8 why=copy`
///     ——`dst` 是那一页**最后 8 字节**、`len = 10` 跨了页 ⇒ `dl = 8`；源那一片 `0x4f0` 装得下
///     10 ⇒ 第一轮 `n = 8`，第二轮源已空 ⇒ `false`。**两边的映射都是好的**（两条 `whole` 都过），
///     被扔掉的是**一条完全正常的报**：名册对结盟服务那一句答复就此消失
///     （`principal: call deny=recv:-6`），连锁把那两台判死
///     （`coalition: no identity plate` / `hub: no league plate`）。
///   · **源那一片更短** ⇒ 搬完源那一片时**两个游标都已推进**，接着搬就等于**跳过一段、把后面的
///     字节落错位置**，而这一手仍答 `true`。症状正是本仓那条老话"长度与发送者都对、**内容不对**"
///     （客侧 `recv-unread`）——debug 档里量到过持树者**一连八次**读到一位客人推来的 52 字节而
///     解不动（`RecvFail::Unread` 那一格带长度就是为它加的）。**如实记**：那八次是**改前**的镜像
///     上量的，字节没有当场照出来（那一版镜像不再复现），故"就是这一支"是**推断**、不是判决。
///
/// 修法就是把账算对：**两个游标各带"这一片段还剩几字节没搬"**，每次只推进搬掉的那 `n`
/// ——片段边界对不齐不再有任何后果。
pub(crate) fn copy(src: &Space, src_va: usize, dst: &Space, dst_va: usize, len: usize) -> bool {
    if !whole(src, src_va, len, PteFlags::R) || !whole(dst, dst_va, len, PteFlags::W) {
        return false;
    }
    let mut from = src.segments(VirtAddr::from_raw(src_va), len);
    let mut into = dst.segments(VirtAddr::from_raw(dst_va), len);
    // 两侧游标：`(本片段当前物理地址, 本片段还剩几字节)`——**没搬完的片段不换下一片**
    // （照实记：旧正文每次都换，于是边界一错开就错到底）。
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
