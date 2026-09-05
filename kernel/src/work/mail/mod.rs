// 任务间通信（mail）— Hole/Pole 两通道 + 门闩 + 授权谱系。
//
//   pie.rs    — 门闩类型（Pie<M>, AnyPie, MailError, HOLE_MSG_LEN）
//   memo.rs   — 全局资源备忘（id → Arc<Meta>）
//   hole.rs   — Hole 数据面（数据过内核，单槽缓冲）
//   pole.rs   — Pole 数据面（页级安全内存，物理帧 + 视图）
//   accord.rs — 转授子集给其他 Task
//   narrow.rs — 就地单调收窄本 pie 权限
//   revoke.rs — 收回授与他人的副本
//
// 数据面不感知 rights；门闩在 envcall 入口 dispatch 时检查。
// 阻塞语义在调度域 wait/wake，mail 不重造调度器。
//
// 权限四元：READ / WRITE / VEST / BACK（单一真相在 `ubi::Permission`）。
// 用户句柄 = per-pie token（全局唯一），envcall 以 token 寻址。

pub mod accord;
pub mod hole;
pub mod memo;
pub mod narrow;
pub mod pie;
pub mod pole;
pub mod revoke;

pub use pie::{AnyPie, MailError, Permission, HOLE_MSG_LEN};

use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::Space;

/// 从用户空间读 `dst.len()` 字节进内核缓冲：逐段翻译（`Space::segments`）+
/// 拷贝；任一段权限缺 R 或越界 → false（不部分写入）。
pub(crate) fn copy_in(space: &Space, dst: &mut [u8], va: usize) -> bool {
    let mut off = 0;
    for (pa, flags, l) in space.segments(VirtAddr::from_raw(va), dst.len()) {
        if !flags.intersects(PteFlags::R) || off + l > dst.len() {
            return false;
        }
        // SAFETY: pa 为恒等映射物理地址；l 在段界与 dst 剩余长度内。
        unsafe {
            core::ptr::copy_nonoverlapping(
                pa.as_usize() as *const u8,
                dst.as_mut_ptr().add(off),
                l,
            );
        }
        off += l;
    }
    off == dst.len()
}

/// 从内核缓冲写 `src.len()` 字节进用户空间：逐段翻译 + 拷贝；任一段权限缺 W
/// 或越界 → false。
pub(crate) fn copy_out(space: &Space, src: &[u8], va: usize) -> bool {
    let mut off = 0;
    for (pa, flags, l) in space.segments(VirtAddr::from_raw(va), src.len()) {
        if !flags.intersects(PteFlags::W) || off + l > src.len() {
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr().add(off), pa.as_usize() as *mut u8, l);
        }
        off += l;
    }
    off == src.len()
}
