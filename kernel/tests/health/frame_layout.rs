//! 受控帧布局：为「普通来源自然形成 2 MiB 大叶」的验收用例塑形帧池。
//!
//! 两件事，都用**正常分配、按同一 layout 归还**的裸块持有：
//!
//! - [`Reserved::preheat`]：从全局块池取走两档元数据尺寸类的小块，但每页保留一块、
//!   其余当场归还。那些页因此停在「至少一块在用」上，不会被块池交还帧池；
//!   物化期间的元数据分配于是不再向帧池取页——否则每 64 页会插进一块元数据页，
//!   相对连续段就被打断。
//! - [`Reserved::drain`]：把帧池里小于 8 MiB 的伙伴块整块取出并持有。此后最小的一次
//!   帧分配来自一段相对连续的 ≥8 MiB 块（≥2048 页），预热那几页取用之后仍远长于
//!   一段 2 MiB 对齐窗所需的 1023 页；一次 1024 页的普通物化因此整段连续。
//!
//! 预算是显式的：塑形最多额外占用 16 MiB 帧池、最多持有 1024 块。任一步失败都返回
//! 错误，已持有的块由 [`Reserved`] 析构时整批归还，不留半个塑形。

#![cfg(all(debug_assertions, feature = "mega-leaf"))]

use alloc::alloc::{alloc, dealloc};
use alloc::vec::Vec;
use core::alloc::Layout;
use core::ptr::NonNull;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::flat::allocator::frame;

/// 相对连续段的最小幂：`2^11` 页 = 8 MiB。
///
/// 为什么不取 `2^10`（4 MiB）：一段 4 MiB 块正好 1024 页，而预热会从块首取走几页，
/// 剩下的段就短于“2 MiB 窗（512 页）＋ 对齐偏移（最多 511 页）”所需的 1023 页。
const RUN_POWER: usize = 11;
/// 塑形最多额外占用的帧池字节数。
const BUDGET_BYTES: usize = 16 * 1024 * 1024;
/// 持有表的最大块数（预算之外的第二道上界，也是持有表预留容量）。
const MAX_BLOCKS: usize = 1024;
/// 预热的两档元数据尺寸类与每档块数：`Arc<Frame>` 控制块要够一次 1024 页物化，
/// 来源驻留表项只要够几次登记。
const HEATED: [(usize, usize); 2] = [(6, 2048), (7, 64)];
/// 预热的**尺寸类数**：净额断言用它给出「每档一张 spare 空页」的上界，从 [`HEATED`] 派生。
pub(crate) const HEATED_CLASSES: usize = HEATED.len();

/// 布局塑形的持有集；析构时把每一块按原 layout 归还。
#[derive(Default)]
pub(crate) struct Reserved {
    blocks: Vec<(NonNull<u8>, Layout)>,
    bytes: usize,
}

impl Reserved {
    /// 当前持有集的大小与字节数（诊断读数用）。
    pub(crate) fn held(&self) -> (usize, usize) {
        (self.blocks.len(), self.bytes)
    }

    /// 预热块池的两档元数据尺寸类：每页保留一块，其余当场归还。
    ///
    /// 保留的是那一页的**第一块**：块池一次取自帧池的一页会被连续发放，故同页的块
    /// 在分配序列里相邻。归还后这些页的「在用块数」不会归零，块池就不会把它抽回帧池。
    pub(crate) fn preheat(&mut self) -> Result<(), &'static str> {
        self.blocks
            .try_reserve(MAX_BLOCKS)
            .map_err(|_| "reserve the shaping table")?;
        for (class, count) in HEATED {
            let layout = Layout::from_size_align(1 << class, 16).map_err(|_| "metadata layout")?;
            let mut addresses = Vec::new();
            addresses
                .try_reserve_exact(count)
                .map_err(|_| "reserve the metadata record")?;
            for _ in 0..count {
                // `alloc` 失败由分配器 shim 直接 abort（`GlobalAllocator::allocate` 返回
                // `Result`，没有 null 表示），这里拿到的必然非空。
                let block = unsafe { alloc(layout) };
                addresses.push(NonNull::new(block).expect("frame alloc aborts on failure"));
            }
            let mut page = usize::MAX;
            for block in addresses {
                let at = block.as_ptr() as usize & !(PAGE_SIZE - 1);
                if at == page {
                    unsafe { dealloc(block.as_ptr(), layout) };
                } else {
                    page = at;
                    self.blocks.push((block, layout));
                    self.bytes += layout.size();
                }
            }
        }
        Ok(())
    }

    /// 抽干帧池里小于 8 MiB 的伙伴块；返回前最低非空桶的幂 ≥ [`RUN_POWER`]。
    pub(crate) fn drain(&mut self) -> Result<(), &'static str> {
        loop {
            let power = frame::lowest_free_power().ok_or("the frame pool has no free block")?;
            if power >= RUN_POWER {
                return Ok(());
            }
            let size = PAGE_SIZE << power;
            if self.bytes + size > BUDGET_BYTES || self.blocks.len() >= MAX_BLOCKS {
                return Err("the frame layout budget is exhausted");
            }
            let layout = Layout::from_size_align(size, PAGE_SIZE).map_err(|_| "run layout")?;
            let block = frame::allocator()
                .allocate(layout)
                .map_err(|_| "allocate a frame block")?;
            let block = NonNull::new(block.as_ptr() as *mut u8).ok_or("frame block pointer")?;
            self.blocks.push((block, layout));
            self.bytes += size;
        }
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        for (block, layout) in self.blocks.drain(..) {
            unsafe { dealloc(block.as_ptr(), layout) };
        }
    }
}
