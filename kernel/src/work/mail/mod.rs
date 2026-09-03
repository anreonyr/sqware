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

use core::ptr::NonNull;

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::lock::SpinLock;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::MapError;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::work::unit::space::{Seg, Space, Span};

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

// ── 共享内存邮路共享结构（dock / ring 共用）───────────────────────

/// dock / ring 共享内存邮路的共享部分：物理帧 + 视图登记。Drop 自动还帧 +
/// 还视图（视图登记的 Weak<Space> 失效 = 空间已 drop；升级成功则调 Space::release
/// 归还 user 段）。
///
/// 抽象动机：dock 与 ring 在「借映共享物理块 + 视图回收」上是同一机制——本
/// 类型把它收敛，mail 仅按需扩展（dock 多 pier/quay 计数、ring 仅 Live/Dead
/// 状态）于各自 Meta 中。
pub(crate) struct SharedBuf {
    /// 共享区物理连续块首地址（恒等映射下 VA 即 PA）。
    pub(crate) base: NonNull<u8>,
    /// 共享区字节数（页对齐向上）。
    pub(crate) bytes: usize,
    /// 各持有 space 的视图（弱引用 + 段区间）——drop 时逐视图归还 user 段。
    /// 弱引用不拖住地址空间生命周期；Arc<Space> 已死则视图随 Space drop 消失。
    /// 锁序：exempt（SpinLock::new）——视图是 mail 私有数据，保护它的锁不参与
    /// 层级校验。
    pub(crate) views: SpinLock<Vec<(Weak<Space>, Span)>>,
}

impl SharedBuf {
    /// 把共享物理块借用映射进 `space`（帧空 = 借用；VA 出 user 段），并登记
    /// 视图。同一 space 重复映射 → 复用既有视图（不重复取段）。
    pub(crate) fn map_into(&self, space: &Arc<Space>) -> Result<usize, MapError> {
        // 同 space 复用：open 方两端同空间，第二次 map_shared 不重复取段。
        {
            let views = self.views.lock();
            if let Some((_, span)) = views
                .iter()
                .find(|(w, _)| w.upgrade().is_some_and(|s| Arc::ptr_eq(&s, space)))
            {
                return Ok(span.va.as_usize());
            }
        }
        let size = self.bytes.next_multiple_of(PAGE_SIZE);
        let flags =
            PteFlags::V | PteFlags::R | PteFlags::W | PteFlags::U | PteFlags::A | PteFlags::D;
        // 模式同窗口（取段 + 借帧装配）；不收回窗口类型——见 window/mod.rs 注。
        let va = space.with_flush(|inner| {
            let va = inner.allocate(Seg::User, size)?;
            inner.borrow_map(
                va,
                PhysAddr::from_raw(self.base.as_ptr() as usize),
                size,
                flags,
            )?;
            Ok::<_, MapError>(va)
        })?;
        self.views
            .lock()
            .push((Arc::downgrade(space), Span::new(Seg::User, va, size, None)));
        Ok(va.as_usize())
    }

    /// 从 frame 分配器取 `bytes` 大小的物理连续块（页对齐），清零后构造。
    /// 帧类别 = Task（任务生命周期，关机归零）。
    pub(crate) fn allocate(bytes: usize) -> Result<Self, MapError> {
        let layout = core::alloc::Layout::from_size_align(bytes, PAGE_SIZE)
            .map_err(|_| MapError::NotAligned)?;
        let ptr = crate::tag!(
            Task,
            frame::allocator()
                .allocate(layout)
                .map_err(|_| MapError::OutOfMemory)?
        );
        // SAFETY: 分配返回的切片指针非空；转成字节指针（长度无关，仅取首址）。
        let base = unsafe { NonNull::new_unchecked(ptr.as_ptr().cast::<u8>()) };
        // SAFETY: 刚分配的合法块（fresh），全长度可写；清零初始化共享区。
        unsafe { core::ptr::write_bytes(base.as_ptr(), 0, bytes) };
        Ok(Self {
            base,
            bytes,
            views: SpinLock::new(Vec::new()),
        })
    }
}

impl Drop for SharedBuf {
    fn drop(&mut self) {
        // 1. 逐视图归还 user 段（取空后放锁再回收——Space::release 经 Space 锁，
        //    不得在 L3 持锁内调用）。
        let views: Vec<(Weak<Space>, Span)> = core::mem::take(&mut *self.views.lock());
        for (weak, span) in views {
            if let Some(space) = weak.upgrade() {
                space.release(span).expect("release: span mismatch");
            }
            // upgrade 失败 = Arc<Space> 已死 → 空间已 drop，视图随 Space drop 消失。
        }
        // 2. 共享区归还 frame 池（Arc 归零 = 双端全 drop）。
        let layout = core::alloc::Layout::from_size_align(self.bytes, PAGE_SIZE)
            .expect("shared frame layout valid");
        // SAFETY: base/bytes 与 allocate 时同源（Layout 可复原）。
        unsafe {
            frame::allocator().deallocate(self.base, layout);
        }
    }
}
