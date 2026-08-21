// 时间模块（time）— 全内核时间域：时钟源（原 clock）+ 计时触发（原 timer）两段
//
// 段一「时钟源」只回答「现在几点了 / 过了多久」：时间读数 + Duration 换算 + tick
// 基准（HERTZ/CYCLE，init 注入后只读；不存任何 deadline 语义）。
// 段二「计时触发」管 tock 日程：tick 节拍计数 + tock/untock/next_tock/drain（TimerHeap
// 锁内真值 + 锁外最近镜像）。段二依赖段一（now/换算/Instant），段一不反向依赖段二
// ——合并后为同文件内部引用。
//
// 换算纪律：模块边界一律 core::time::Duration；内部热路径 raw u64 ticks；换算用
// u128 中间量、饱和防溢出，无浮点。

use alloc::collections::binary_heap::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::Reverse;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use riscv::register::time;

use crate::lock::OnceLock;
use crate::lock::{Level, SpinLock};
use crate::machine;

const NANOS_PER_SEC: u128 = 1_000_000_000;

// ── 段一：时钟源（原 clock）────────────────────────────

/// 单调时刻：time CSR 刻度（u64 计数器）薄包装。
///
/// 时间区间（Duration）在模块边界折算；本类型用于调度器/驱动内部比较与
/// 「语义时间点」传递。回绕安全：差值一律 wrapping/saturating 减法。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

/// 时钟初始化错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockError {
    /// DTB 未报告 timebase-frequency（或为 0）。
    NoTimebase,
    /// 重复初始化。
    AlreadyInit,
}

/// init 注入后只读。
static HERTZ: OnceLock<u64> = OnceLock::new();
/// 启动时刻的计数器读数（uptime 基准）。
static CYCLE: AtomicU64 = AtomicU64::new(0);

/// 初始化时钟：注入 hertz ，记录启动时刻。
///
/// 必须在任何时间 API 调用之前、且在 trap 武装（runtime::init）之前调用。
///
/// # Errors
///
/// hertz 为 0（DTB 缺失）→ ClockError::NoTimebase；重复初始化 →
/// ClockError::AlreadyInit。
pub fn init() -> Result<(), ClockError> {
    let hertz = machine::info().hertz;
    if hertz == 0 {
        return Err(ClockError::NoTimebase);
    }
    HERTZ
        .set(hertz as u64)
        .map_err(|_| ClockError::AlreadyInit)?;
    CYCLE.store(time::read() as u64, Ordering::Relaxed);
    Ok(())
}

fn hertz() -> u64 {
    HERTZ.get().copied().expect("clock not initialized")
}

/// 当前时刻（time CSR 计数器刻度）。
pub fn now() -> Instant {
    Instant(time::read() as u64)
}

/// 自启动以来的单调时长（uptime）。
pub fn uptime() -> Duration {
    let boot = CYCLE.load(Ordering::Relaxed);
    ticks_to_duration((time::read() as u64).wrapping_sub(boot))
}

impl Instant {
    /// 内部刻度（计时触发段的 WFI/arm 目标、热路径比较用）。
    pub fn as_ticks(self) -> u64 {
        self.0
    }

    /// 从刻度构造时刻（计时触发段 next_tock 还原用）。
    pub(crate) fn from_ticks(t: u64) -> Instant {
        Instant(t)
    }

    /// 自 earlier 以来的时长（回绕安全；earlier 在未来按零处理）。
    #[allow(dead_code)] // 预留：syscall clock_gettime / 驱动超时统计用
    pub fn elapsed_since(&self, earlier: Instant) -> Duration {
        ticks_to_duration(self.0.wrapping_sub(earlier.0))
    }

    /// 自本时刻以来经过的时长（= now − self）。
    #[allow(dead_code)] // 预留：uptime 之外的相对计时用
    pub fn elapsed(&self) -> Duration {
        now().elapsed_since(*self)
    }

    /// 本时刻 + Duration（换算饱和防溢出；语义为「最晚到期时刻」）。
    pub fn add(&self, d: Duration) -> Instant {
        Instant(self.0.wrapping_add(duration_to_ticks(d)))
    }

    /// 本时刻 − Duration（换算饱和防溢出；非负语义由调用方保证，wrapping 承担）。
    #[allow(dead_code)] // 预留：IPC 超时 / 协议 deadline 计算用
    pub fn sub(&self, d: Duration) -> Instant {
        Instant(self.0.wrapping_sub(duration_to_ticks(d)))
    }

    /// 自 earlier 以来的时长；earlier 在未来 → None。
    #[allow(dead_code)] // 预留：测试与诊断用
    pub fn checked_duration_since(&self, earlier: Instant) -> Option<Duration> {
        (self.0 >= earlier.0).then_some(ticks_to_duration(self.0 - earlier.0))
    }
}

/// ticks → Duration（u128 中间量、饱和到 Duration 可表达范围）。
/// pub(crate)：供计时触发段与 trap 使用。
pub(crate) fn ticks_to_duration(ticks: u64) -> Duration {
    let ns = (ticks as u128).saturating_mul(NANOS_PER_SEC) / hertz() as u128;
    Duration::from_nanos(ns.min(u64::MAX as u128) as u64)
}

/// Duration → ticks（u128 中间量、饱和到 u64::MAX）。
/// pub(crate)：供计时触发段与 trap 使用。
pub(crate) fn duration_to_ticks(d: Duration) -> u64 {
    let ns = d.as_nanos();
    let t = ns.saturating_mul(hertz() as u128) / NANOS_PER_SEC;
    t.min(u64::MAX as u128) as u64
}

// ── 段二：计时触发（原 timer）───────────────────────────

/// 镜像的无 tock 哨兵（内部；对外以 next_tock() -> None 表达）。
const NONE: u64 = u64::MAX;

/// 节拍计数（ENV_GET_TICKS 兼容）。
static TICKS: AtomicU64 = AtomicU64::new(0);

/// tock 堆 — 全局一份。内层：锁内真值；锁外：最近 tock 原子镜像。
/// 与 scheduler::Scheduler 的内层模式同构：镜像由持锁方法 recompute_nearest
/// 派生，唯一修改路径在 Inner 内——不变量靠构造成立，无独立静态失步面。
struct TimerHeap {
    /// 锁内真值：堆 + 惰性取消集（同一把 level-3 锁内维护）。
    inner: SpinLock<TimerInner>,
    /// 最近未取消 tock 镜像（AcqRel 与 recompute_nearest 的 Release 配对；
    /// u64::MAX = 无）。供 WFI/scheduler 锁外读。
    nearest: AtomicU64,
}

struct TimerInner {
    heap: BinaryHeap<Reverse<(u64, u64)>>,
    cancelled: Vec<u64>,
}

static TIMER_HEAP: TimerHeap = TimerHeap {
    inner: SpinLock::new_level(Level::L3, TimerInner {
        heap: BinaryHeap::new(),
        cancelled: Vec::new(),
    }),
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

// ── 节拍计数（ENV_GET_TICKS 兼容；trap 定时器分支驱动）───

/// 定时器中断发生一次（返回递增后的计数；trap 分支调用）。
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
#[allow(dead_code)] // 预留：IPC 超时等待 / 带超时锁被提前唤醒时取消用
pub fn untock(handle: u64) {
    let mut i = TIMER_HEAP.inner.lock();
    if !i.cancelled.contains(&handle) {
        i.cancelled.push(handle);
    }
    TIMER_HEAP.recompute_nearest(&i);
}

/// 最近一个未到点 tock（锁外原子读；None = 无）。
#[allow(dead_code)] // 预留：驱动/调度轮询最近唤醒点
pub fn next_tock() -> Option<Instant> {
    let t = TIMER_HEAP.peek_nearest();
    (t != NONE).then_some(Instant::from_ticks(t))
}

/// 取出全部已到点 tock 的句柄（trap 定时器分支 / 空闲核 WFI 后驱动唤醒）。
///
/// 返回的句柄由调用方（scheduler::unpark）在放锁后处理——本函数不持任何
/// 调度器锁，只在 TIMER_HEAP 锁内完成弹堆与镜像刷新。
pub fn drain(now: Instant) -> Vec<u64> {
    let mut due = Vec::new();
    let mut i = TIMER_HEAP.inner.lock();
    let now = now.as_ticks();
    while let Some(Reverse((t, _))) = i.heap.peek() {
        if *t > now {
            break;
        }
        let Reverse((_, handle)) = i.heap.pop().expect("peeked non-empty heap entry");
        if i.cancelled.contains(&handle) {
            i.cancelled.retain(|c| *c != handle);
            continue;
        }
        due.push(handle);
    }
    TIMER_HEAP.recompute_nearest(&i);
    due
}
