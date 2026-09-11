// 任务间通信（mail）— Hole/Pole/Nole 三种数据面的资源实体 + IPC 数据面。
//
// 与 `unit::gate` 的分工：mail 只持**资源实体**（HoleMeta/PoleMeta）+ IPC 数据面
// （push/pull/map/unmap）+ 用户空间拷贝（copy_in/out）；
// 能力模型（Pie/AnyPie/授权）在 `unit::gate`（gate 单向依赖 mail）。
//
//   hole.rs   — Hole 数据面（数据过内核，单槽缓冲）+ meta()
//   pole.rs   — Pole 数据面（页级安全内存，物理帧 + 视图）+ meta()
//   nole.rs   — Nole **无数据面**（只有身份与存活）——存在权的载体
//
// **没有全局资源表**：资源寿命 = 能力寿命（门闩持唯一的强引用 `Arc<Meta>`），
// 最后一份门闩消失即回收。
//
// 数据面不感知 rights；门闩在 envcall 入口 dispatch 时检查。
// 阻塞语义在调度域 wait/wake，mail 不重造调度器。
//
// 三者按"有没有数据面"分：Hole 有槽、Pole 有页、Nole **什么都没有**——故 Nole 是
// 唯一能被用作"存在权"载体的类型（资源权需要资源，存在权不需要）。
//
// 权限四元：READ / WRITE / VEST / BACK（单一真相在 `env::Permission`）。
// 用户句柄 = per-pie token（全局唯一），envcall 以 token 寻址。

pub mod hole;
pub mod pole;
pub mod nole;

// 资源实体类型 re-export：`unit::gate` 的 Pie<M> 泛型直指它们（gate → mail 单向依赖）。
pub(crate) use hole::HoleMeta;
pub(crate) use pole::PoleMeta;

/// Hole 单消息字节数上限（每 hole unseal 时定 mtu ∈ [1, HOLE_MTU_MAX]；槽缓冲按
/// mtu 在 HoleMeta 内预分配）。原 HOLE_MSG_LEN 的固定 64B 形态由调用方选 mtu=64
/// 等价复现——dispatch 协议沿用 64B 不变。
pub const HOLE_MTU_MAX: usize = 4096;

use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::Space;

/// 区间可整段拷吗：`[va, va+len)` 的**每一个**段都在、权限含 `need`、
/// 且段长之和恰好 `len`（中途未映射页 ⇒ [`Segments`] 提前终止 ⇒ 和不等于）。
///
/// 这是"要么全写要么不写"的**前置**：两半共用它，先整段验完再动一个字节。
/// 代价是区间多走一遍 [`Segments`]（逐页 `translate`）；64 B 的消息通常只落在
/// 一两页内，可接受。用户裁决：**先整段验完，再动一个字节**。
///
/// 注：两遍之间映射可能变（他核 unmap）——那是**既有**窗口（单遍实现同样逐页
/// 取放 Space 锁），不是本契约引入的；先验后写只是让"失败"不再留下半截数据。
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

/// 从用户空间读 `dst.len()` 字节进内核缓冲：逐段翻译（`Space::segments`）+
/// 拷贝。任一段权限缺 R / 越界 / 中途未映射 ⇒ false。
///
/// **契约：「要么全读，要么 `dst` 一个字节都不动」**——先 [`whole`] 整段验完
/// 再拷。内核缓冲（`staging`）拿到半截数据本不外泄，但"读失败却改过调用方的
/// 缓冲"这种假契约不留（见 [`copy_out`] 的同一句话）。
pub(crate) fn copy_in(space: &Space, dst: &mut [u8], va: usize) -> bool {
    if !whole(space, va, dst.len(), PteFlags::R) {
        return false;
    }
    let mut off = 0;
    for (pa, _, l) in space.segments(VirtAddr::from_raw(va), dst.len()) {
        // SAFETY: pa 为恒等映射物理地址；`whole` 已验本段权限含 R、且 l 在段界
        // 与 dst 剩余长度内（同一区间、同一遍历器，两遍之间只可能变窄）。
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

/// 从内核缓冲写 `src.len()` 字节进用户空间：逐段翻译 + 拷贝。任一段权限缺 W /
/// 越界 / 中途未映射 ⇒ false。
///
/// **契约：「要么全写，要么用户缓冲区一个字节都不动」**——先 [`whole`] 整段验完
/// 再拷。旧版是在**写的过程中**逐段判权限，于是失败路径上**前面的页已经写进用户
/// 缓冲区了**（`mail/mod.rs` 那句"不部分写入"因此是假话，B2）；今天所有调用方都
/// 传精确长度的缓冲、掩盖着这个假契约。
pub(crate) fn copy_out(space: &Space, src: &[u8], va: usize) -> bool {
    if !whole(space, va, src.len(), PteFlags::W) {
        return false;
    }
    let mut off = 0;
    for (pa, _, l) in space.segments(VirtAddr::from_raw(va), src.len()) {
        // SAFETY: pa 为恒等映射物理地址；`whole` 已验本段权限含 W、且长度在界内。
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr().add(off), pa.as_usize() as *mut u8, l);
        }
        off += l;
    }
    true
}
