// 堆页所有权位图 — debug 专用：追踪「哪些物理页被 block 堆持有」
//
// 目的：定位「活堆页泄漏进 frame 池」的时刻。症状链（实测）：
//   某块堆页被错误归还 frame → frame 把它交给任务栈/trap 帧/另一堆页 →
//   两个持有者同时写同一物理页 → Arc<Task> 头、freepool 链表、vtable 被覆写
//   → strong_count 幻值、双 free、内核缺页。
// 位图只回答一个问题：这一页现在是不是**堆持有**？frame 侧在分配/归还的前后
// 各查一次：命中堆持页 = 首次越界点，panic 抓现行。
//
// 实现：无锁位图（AtomicU64 静态数组）——block 侧在 BLOCK 锁内、frame 侧在
// FRAME 锁内访问，跨锁不嵌套（位操作原子，无死锁面）。索引相对 frame 区域
// 基址（frame::init 写入 FRAME_BASE，block 首次 refill 必然晚于 frame::init，
// 读到的是稳定值）。
//
// 生命周期：refill 取页 → set（前置断言未持有）；decrease_used 还页 → clear
// （前置断言持有）。frame::allocate 弹出页 → assert 非持有；frame::deallocate
// 推入页 → assert 非持有。release 构建整体编译为空。

#![cfg_attr(not(debug_assertions), allow(dead_code))]

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// frame 区域基址（物理页号计算的基准；frame::init 写入，此后只读）。
#[cfg(debug_assertions)]
static FRAME_BASE: AtomicUsize = AtomicUsize::new(0);

/// 页数覆盖：128 MiB / 4 KiB = 32768 页 = 512 × u64（远大于 QEMU 默认内存）。
#[cfg(debug_assertions)]
const WORDS: usize = 512;

#[cfg(debug_assertions)]
static BITS: [AtomicU64; WORDS] = [const { AtomicU64::new(0) }; WORDS];

#[cfg(debug_assertions)]
fn idx(pa: usize) -> usize {
    let base = FRAME_BASE.load(Ordering::Relaxed);
    let n = (pa - base) / 0x1000;
    assert!(
        n < (WORDS * 64) as usize,
        "pageown: page {pa:#x} outside bitmap (base {base:#x}, idx {n})"
    );
    n
}

/// frame::init 记录区域基址（恰好一次，单 hart boot 期）。
#[cfg(debug_assertions)]
pub(crate) fn set_base(base: usize) {
    FRAME_BASE.store(base, Ordering::Relaxed);
}

/// 该页当前是否被 block 堆持有。
#[cfg(debug_assertions)]
pub(crate) fn is_held(pa: usize) -> bool {
    let n = idx(pa);
    BITS[n / 64].load(Ordering::Relaxed) & (1u64 << (n % 64)) != 0
}

/// 标记页为堆持有（refill 取页时）——前置断言：此前未持有（页来自 frame 池）。
#[cfg(debug_assertions)]
pub(crate) fn hold(pa: usize) {
    let n = idx(pa);
    assert!(
        !is_held(pa),
        "block refill: page {pa:#x} already owned by block heap (double refill)"
    );
    BITS[n / 64].fetch_or(1u64 << (n % 64), Ordering::Relaxed);
}

/// 解除页的堆持有（decrease_used 整页归还时）——前置断言：确实持有。
#[cfg(debug_assertions)]
pub(crate) fn release(pa: usize) {
    let n = idx(pa);
    assert!(
        is_held(pa),
        "block decrease_used: page {pa:#x} not owned by block heap (stale/foreign page?)"
    );
    BITS[n / 64].fetch_and(!(1u64 << (n % 64)), Ordering::Relaxed);
}

/// frame 侧检查：分配/归还前该页**不得**被堆持有（命中 = 活堆页泄漏进 frame 池）。
#[cfg(debug_assertions)]
pub(crate) fn assert_not_held(pa: usize, ctx: &str) {
    assert!(
        !is_held(pa),
        "frame {ctx}: page {pa:#x} still owned by block heap — live heap page leaked into frame pool"
    );
}