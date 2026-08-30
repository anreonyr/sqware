// 任务间通信（mail）— 三通道（unit / room 之后的第三域）。
//
//   port.rs — 内核邮路：消息拷贝经内核（定长小消息、单槽）；
//   dock.rs — 多对一共享内存邮路（多 pier 生产 / 唯一 quay 消费，零拷贝）；
//   ring.rs — 一对一共享内存邮路（两端固定，零拷贝）。
//
// 词族：open/shut/push/pull 成对；阻塞统一走调度域 wait/wake（conductor 词族）
// ——mail 消费调度器，不重造调度器。
//
// 首版作用域（硬不变量）：两端同空间（同一 team）——wait/wake 键经 envcall 合成
// 空间身份（WaitKey::compose，asid||va）。跨空间通道需重定键身份，后话。

use crate::memory::manager::addr::VirtAddr;
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::Space;

pub mod dock;
pub mod port;
pub mod ring;

/// 从用户空间读 `dst.len()` 字节进内核缓冲：逐段翻译（`Space::segments`）+
/// 拷贝；任一段权限缺 R 或越界 → false（不部分写入）。
pub(crate) fn copy_in(space: &Space, dst: &mut [u8], va: usize) -> bool {
    let mut off = 0;
    for (pa, flags, l) in space.segments(VirtAddr::from_raw(va), dst.len()) {
        if !flags.intersects(PteFlags::R) || off + l > dst.len() {
            return false;
        }
        // SAFETY: pa 为恒等映射物理地址（教学内核直接解引用）；l 在段界与
        // dst 剩余长度内。
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
        // SAFETY: 同 copy_in；写侧须含 W（段权限在 segments 中即为叶子 PTE 权限）。
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr().add(off), pa.as_usize() as *mut u8, l);
        }
        off += l;
    }
    off == src.len()
}
