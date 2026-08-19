// 堆页所有权位图 — debug 专用：追踪「哪些物理页被 block 堆持有」
//
// 目的：定位「活堆页泄漏进 frame 池」的时刻。症状链（实测）：
//   某块堆页被错误归还 frame → frame 把它交给任务栈/trap 帧/另一堆页 →
//   两个持有者同时写同一物理页 → Arc<Task> 头、freepool 链表、vtable 被覆写
//   → strong_count 幻值、双 free、内核缺页。
// 位图只回答一个问题：这一页现在是不是**堆持有**？frame 侧在分配/归还的前后
// 各查一次：命中堆持页 = 首次越界点，panic 抓现行。
//
// 实现：无锁位图（AtomicU64 数组）——block 侧在 BLOCK 锁内、frame 侧在
// FRAME 锁内访问，跨锁不嵌套（位操作原子，无死锁面）。索引相对 frame 区域
// 基址（block::init 写入 FRAME_BASE，block 首次 refill 必然晚于分配，读到的是
// 稳定值）。
//
// 存储：按实际空闲区页数动态分配（内存更大时帧分配器会合法给出更远的页）。
// 数组在 block::init 阶段（frame base 定址之前）从 bump 分配：必须落在 frame
// 区域之下——否则帧分配器会把它当作可用页交出，堆数据覆写位图（word 突然归零
// → 误报/漏报持有关系）。
//
// 生命周期：refill 取页 → set（前置断言未持有）；decrease_used 还页 → clear
// （前置断言持有）。frame::allocate 弹出页 → assert 非持有；frame::deallocate
// 推入页 → assert 非持有。release 构建整体编译为空。

#![cfg_attr(debug_assertions, allow(dead_code))]

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use alloc::boxed::Box;

use crate::lock::OnceLock;

/// frame 区域基址（物理页号计算的基准；block::init 写入，此后只读）。
#[cfg(debug_assertions)]
static FRAME_BASE: AtomicUsize = AtomicUsize::new(0);

/// 堆页所有权位图本体 — 按实际空闲区页数动态分配（每 64 页一个字）。
#[cfg(debug_assertions)]
static BITS: OnceLock<Box<[AtomicU64]>> = OnceLock::new();

/// 位图覆盖页数（BITS 长度 × 64 位）。
#[cfg(debug_assertions)]
fn bit_count() -> usize {
    BITS.get().expect("pageown not initialized").len() * 64
}

#[cfg(debug_assertions)]
fn idx(pa: usize) -> usize {
    let base = FRAME_BASE.load(Ordering::Relaxed);
    let n = (pa - base) / 0x1000;
    assert!(
        n < bit_count(),
        "pageown: page {pa:#x} outside bitmap (base {base:#x}, idx {n})"
    );
    n
}

/// 记录区域基址并分配位图（block::init 调用恰好一次，单 hart boot 期，
/// 先于 frame base 定址，保证数组落在帧区之下）。
///
/// `pages` = 实际空闲区页数（`(edge - base) / PAGE_SIZE`）；位图按 64 页/字
/// 向上取整分配，允许覆盖整个帧区（无论 RAM 多大）。
#[cfg(debug_assertions)]
pub(crate) fn set_base(base: usize, pages: usize) {
    FRAME_BASE.store(base, Ordering::Relaxed);
    let words = pages.div_ceil(64);
    let table: Box<[AtomicU64]> = (0..words).map(|_| AtomicU64::new(0)).collect();
    assert!(BITS.set(table).is_ok(), "pageown double init");
}

/// 该页当前是否被 block 堆持有。
#[cfg(debug_assertions)]
pub(crate) fn is_held(pa: usize) -> bool {
    let n = idx(pa);
    BITS.get().expect("pageown not initialized")[n / 64].load(Ordering::Relaxed)
        & (1u64 << (n % 64))
        != 0
}

/// 标记页为堆持有（refill 取页时）——前置断言：此前未持有（页来自 frame 池）。
#[cfg(debug_assertions)]
#[track_caller]
pub(crate) fn hold(pa: usize) {
    let n = idx(pa);
    assert!(
        !is_held(pa),
        "block refill: page {pa:#x} already owned by block heap (double refill)"
    );
    BITS.get().expect("pageown not initialized")[n / 64]
        .fetch_or(1u64 << (n % 64), Ordering::Relaxed);
}

/// 解除页的堆持有（decrease_used 整页归还时）——前置断言：确实持有。
#[cfg(debug_assertions)]
#[track_caller]
pub(crate) fn release(pa: usize) {
    let n = idx(pa);
    assert!(
        is_held(pa),
        "block decrease_used: page {pa:#x} not owned by block heap (stale/foreign page?)"
    );
    BITS.get().expect("pageown not initialized")[n / 64]
        .fetch_and(!(1u64 << (n % 64)), Ordering::Relaxed);
}

/// frame 侧检查：分配/归还前该页**不得**被堆持有（命中 = 活堆页泄漏进 frame 池）。
#[cfg(debug_assertions)]
#[track_caller]
pub(crate) fn assert_not_held(pa: usize, ctx: &str) {
    assert!(
        !is_held(pa),
        "frame {ctx}: page {pa:#x} still owned by block heap — live heap page leaked into frame pool"
    );
}

