// 计时模块（timer）— 「到点叫我」：tock 日程 + 节拍计数
//
// 中心意象 tick-tock：tick = 节拍（周期中断计数）；tock = 一个被安排的「到点
// 唤醒」事件。本模块管理者一堆 tock 的日程：登记（tock）、取消（untock）、
// 到期取走（drain）、查最近（next_tock）。
//
// 数据结构：TimerHeap = inner(SpinLock<TimerInner>) + 锁外最近 tock 镜像——镜像
// 由持锁方法 recompute_nearest 派生，唯一修改路径在 Inner 内。锁层级 level 3。
//
// 簿记约定：堆只存 (wake_at, handle) 纯数据，不持任务引用。

use alloc::collections::binary_heap::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::Reverse;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::lock::{Level, SpinLock};
use crate::runtime::chrono::clock::Instant;

/// 镜像的无 tock 哨兵（内部；对外以 next_tock() -> None 表达）。
const NONE: u64 = u64::MAX;

/// 节拍计数（ENV_GET_TICKS 兼容）。
static TICKS: AtomicU64 = AtomicU64::new(0);

/// tock 堆 — 全局一份。内层：锁内真值；锁外：最近 tock 原子镜像——镜像由持锁
/// 方法 recompute_nearest 派生，唯一修改路径在 Inner 内，不变量靠构造成立。
struct TimerHeap {
    /// 锁内真值：堆 + 惰性取消集（同一把 level-3 锁内维护）。
    inner: SpinLock<TimerInner>,
    /// 最近未取消 tock 镜像（AcqRel 与 recompute_nearest 的 Release 配对；
    /// u64::MAX = 无）。供锁外读。
    nearest: AtomicU64,
}

struct TimerInner {
    heap: BinaryHeap<Reverse<(u64, u64)>>,
    cancelled: Vec<u64>,
}

static TIMER_HEAP: TimerHeap = TimerHeap {
    inner: SpinLock::new_level(
        Level::L3,
        TimerInner {
            heap: BinaryHeap::new(),
            cancelled: Vec::new(),
        },
    ),
    nearest: AtomicU64::new(NONE),
};

impl TimerHeap {
    /// 锁外读最近 tock 镜像（Acquire；NONE = 无）。
    fn peek_nearest(&self) -> u64 {
        self.nearest.load(Ordering::Acquire)
    }

    /// 锁内刷新镜像：从内层数据派生最近未取消 tock（须持 inner 锁；Release）。
    fn recompute_nearest(&self, i: &TimerInner) {
        let t = i
            .heap
            .iter()
            .filter(|e| !i.cancelled.contains(&e.0.1))
            .map(|e| e.0.0)
            .min()
            .unwrap_or(NONE);
        self.nearest.store(t, Ordering::Release);
    }
}

// ── 节拍计数（ENV_GET_TICKS 兼容）───

/// 定时器中断发生一次（返回递增后的计数）。
pub fn tick() -> u64 {
    TICKS.fetch_add(1, Ordering::Relaxed) + 1
}

/// 累计定时器中断次数。
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

// ── tock 日程（deadline 注册表）──────────────────────────

/// 在句柄上安排一个到点（tock）事件：入堆 + 刷新最近 tock 镜像。
///
/// 前置：handle 由调度器自管（先入簿、后 tock，闭合「堆可见 ⇒ 簿记必在」）。
pub fn tock(handle: u64, wake_at: u64) {
    let mut i = TIMER_HEAP.inner.lock();
    i.heap.push(Reverse((wake_at, handle)));
    TIMER_HEAP.recompute_nearest(&i);
}

/// 取消这个 tock（惰性：堆项留至到期被 drain 丢弃；此后该句柄不再唤醒任何任务）。
#[allow(dead_code)] // 预留：超时等待被提前唤醒时取消用
pub fn untock(handle: u64) {
    let mut i = TIMER_HEAP.inner.lock();
    if !i.cancelled.contains(&handle) {
        i.cancelled.push(handle);
    }
    TIMER_HEAP.recompute_nearest(&i);
}

/// 最近一个未到点 tock（锁外原子读；None = 无）。
#[allow(dead_code)] // 预留：轮询最近唤醒点
pub fn next_tock() -> Option<Instant> {
    let t = TIMER_HEAP.peek_nearest();
    (t != NONE).then_some(Instant::from_ticks(t))
}

/// 取出全部已到点 tock 的句柄。
///
/// 返回的句柄由调用方在放锁后处理——本函数只在 TIMER_HEAP 锁内完成弹堆与
/// 镜像刷新，不持任何其它锁。
///
/// **锁内零分配**：`due` 用固定栈缓冲（[`MAX_DUE`]）——持 Level::L3 锁时不得触
/// 分配器（分配器锁均 exempt，lockdep 逃检）。到期数超 `MAX_DUE` 的极端洪峰
/// 截断（尽力而为，timer 路径不许失败）。
pub fn drain(now: Instant) -> Vec<u64> {
    const MAX_DUE: usize = 64;
    let mut due: [u64; MAX_DUE] = [0; MAX_DUE];
    let mut n = 0usize;
    let mut i = TIMER_HEAP.inner.lock();
    let now = now.as_ticks();
    while let Some(Reverse((t, _))) = i.heap.peek() {
        if *t > now || n >= MAX_DUE {
            break;
        }
        let Reverse((_, handle)) = i.heap.pop().expect("peeked non-empty heap entry");
        if i.cancelled.contains(&handle) {
            i.cancelled.retain(|c| *c != handle);
            continue;
        }
        due[n] = handle;
        n += 1;
    }
    TIMER_HEAP.recompute_nearest(&i);
    drop(i); // 放锁后再组装结果（锁外分配，零锁内分配）
    let mut out = Vec::with_capacity(n);
    out.extend_from_slice(&due[..n]);
    out
}
