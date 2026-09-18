// 计时模块（timer）— 「到点叫我」：tock 日程 + 节拍计数
//
// 中心意象 tick-tock：tick = 节拍（周期中断计数）；tock = 一个被安排的「到点
// 唤醒」事件。本模块管理者一堆 tock 的日程：登记（tock）、消音（mute）、
// 到期取走（drain）、查最近（due）。另有机器定时器的一拍（beat）。
//
// 数据结构：TimerHeap = inner(SpinLock<TimerInner>) + 锁外最近 tock 镜像——镜像
// 由持锁方法 recompute_nearest 派生，唯一修改路径在 Inner 内。锁层级 level 3。
//
// 簿记约定：堆只存 (wake_at, handle) 纯数据，不持任务引用。

use alloc::collections::binary_heap::BinaryHeap;
use core::cmp::Reverse;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use riscv::register::time;

use crate::lock::{Level, SpinLock};
use crate::runtime::chrono::clock::{self, Instant};
use sbi::{TimerCall, ecall::SArgs, fid::Timer};

/// 镜像的无 tock 哨兵（内部；对外以 due() -> None 表达）。
const NONE: u64 = u64::MAX;

/// **失明上限**——任一核两次「看一眼世界」之间允许的最长时间（毫秒）。
///
/// 语义改名，不是新数：它就是原先抢占量子里写死的那个 `100ms`（`trap.rs`、
/// `hart.rs`、`stack.rs` 三处各写一份）。改名是因为它管的不止"跑多久换人"：
/// 到点登记（`tock`）发生在别的核上时，登记者自己会按 [`beat_until`] 把**本核**
/// 武装点收到最近到点，而失明的核最迟在 `BLIND_MS` 后醒来重算
/// `min(上限, 最近活到点)` ⇒ 自愈。这也是「不变量不会永久破」的来源：破口寿命
/// ≤ `BLIND_MS`。
pub const BLIND_MS: u64 = 100;

/// 忙核的 `ceil`：失明上限的刻度。**一个家**——四处调用点不许各抄一份换算。
pub fn blind_ceiling() -> u64 {
    clock::duration_to_ticks(Duration::from_millis(BLIND_MS))
}

/// 到点兑现迟到的累计读数（只读计数器）。
///
/// 语义：`late = drain 时的 now − 该项登记的 wake_at`（timebase 刻度）。全部
/// Relaxed——只被停机读出口读一次，不承载任何同步。热路径只做原子加/取大，
/// 不分配、不阻塞、不 putln（`drain` 持 `TIMER_HEAP` 锁）。
static LATE_N: AtomicU64 = AtomicU64::new(0);
static LATE_SUM: AtomicU64 = AtomicU64::new(0);
static LATE_MAX: AtomicU64 = AtomicU64::new(0);

/// 节拍计数（ENV_TICKS 兼容）。
static TICKS: AtomicU64 = AtomicU64::new(0);

/// tock 堆 — 全局一份。镜像见模块头注释。
struct TimerHeap {
    /// 锁内真值：堆 + 惰性取消集。
    inner: SpinLock<TimerInner>,
    /// 最近未取消 tock 镜像（u64::MAX = 无）。供锁外读。
    nearest: AtomicU64,
}

struct TimerInner {
    heap: BinaryHeap<Reverse<(u64, u64)>>,
}

static TIMER_HEAP: TimerHeap = TimerHeap {
    inner: SpinLock::new_level(
        Level::L3,
        TimerInner {
            heap: BinaryHeap::new(),
        },
    ),
    nearest: AtomicU64::new(NONE),
};

impl TimerHeap {
    /// 锁外读最近 tock 镜像（Acquire；NONE = 无）。
    fn peek_nearest(&self) -> u64 {
        self.nearest.load(Ordering::Acquire)
    }

    /// 锁内刷新镜像：从内层数据派生最近到点（须持 inner 锁；Release）。
    /// 堆里没有墓碑——`mute` 是真摘除，故不必过滤。
    fn recompute_nearest(&self, i: &TimerInner) {
        let t = i.heap.iter().map(|e| e.0.0).min().unwrap_or(NONE);
        self.nearest.store(t, Ordering::Release);
    }
}

// ── 节拍计数（ENV_TICKS 兼容）───

/// 定时器中断发生一次（返回递增后的计数）。
pub fn tick() -> u64 {
    TICKS.fetch_add(1, Ordering::Relaxed) + 1
}

/// 累计定时器中断次数。
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

// ── 机器定时器（tick 发源）──────────────────────────────

/// 打下一拍：将下一机器定时器中断安排在「当前 time 起 interval 刻度后」：
/// stimecmp = time + interval（SBI Time 扩展，绝对时间）。interval 为
/// timebase 刻度（调用方经 clock::duration_to_ticks 换算）。SBI 失败即
/// panic（定时器路径不许失败，与 drain 同纪律）。
pub fn beat(interval: u64) {
    let next = (time::read() as u64).wrapping_add(interval);
    TimerCall::new(Timer::SetTimer)
        .args(SArgs {
            a0: next as usize,
            ..Default::default()
        })
        .call()
        .unwrap();
}

/// 武装到 `min(ceil, 最近活到点 − 现在)` —— 即「武装点 = min(本核上限, 最近活到点)」。
///
/// 与 [`beat`] 只差**取哪个时刻**：`beat` 认死一个 interval，本函数认 `TIMER_HEAP`
/// 的锁外真值镜像（[`due`]）——于是「武装」与「登记到点」从此相干。`due() == None`
/// （堆空 / 启动期）即退化为 `beat(ceil)`，故三个原 `beat(100ms)` 调用点与今天等价。
///
/// `ceil` 是本核上限：忙核 = [`BLIND_MS`] 的刻度（失明上限），空闲核 = `BEACON_TICK`
/// / `WFI_FAR`。饱和减法：到点已过 ⇒ 0 ⇒ 立刻再来一拍。
///
/// 不变量：任一时刻「本核武装点 ≤ 最近活到点」。`nearest` 只在持 `TIMER_HEAP` 锁时
/// 由 `recompute_nearest` 派生，写点仅 `tock`/`mute`/`drain`；每次武装都从该真值
/// 重算（不沿用上次的绝对点）⇒ 无累积误差、不认上次是谁武装。`min` 只会把武装点
/// **提前**；`due` 变晚/消失只发生在 `mute`/`drain` 摘掉该项时——那意味着该到点已
/// 无需兑现。破口寿命 ≤ `BLIND_MS`（每个武装点都重算 ⇒ 自愈）。
///
/// SBI 失败即 panic，纪律与 [`beat`] 同。
pub fn beat_until(ceil: u64) {
    let delta = match due() {
        Some(t) => t.as_ticks().saturating_sub(time::read() as u64).min(ceil),
        None => ceil,
    };
    beat(delta);
}

// ── tock 日程（deadline 注册表）──────────────────────────

/// 在句柄上安排一个到点（tock）事件：入堆 + 刷新最近 tock 镜像。
///
/// 前置：handle 由调度器自管（先入簿、后 tock，闭合「堆可见 ⇒ 簿记必在」）。
///
/// # Errors
///
/// 堆扩不出来（内存耗尽）→ `Err(())`（与 `try_reserve_roster` 同一口径）。
/// **扩容与入堆在同一把锁内**——挂起路径上这次 `BinaryHeap` 扩容此前是不可失败的，
/// 内存吃紧即整机 halt；现在它是一个返回码，调用方（`wait::block`）当场答 `OoM`，
/// 任务不挂起、机器照旧活着。
pub fn tock(handle: u64, wake_at: u64) -> Result<(), ()> {
    let mut i = TIMER_HEAP.inner.lock();
    i.heap.try_reserve(1).map_err(|_| ())?;
    i.heap.push(Reverse((wake_at, handle)));
    TIMER_HEAP.recompute_nearest(&i);
    Ok(())
}

/// 消音这个 tock —— 与 [`tock`] 互为逆操作：堆里那一项直接摘掉，此后该句柄不再
/// 唤醒任何任务。不在堆里（已被 drain 取走）即 no-op —— 没有惰性标记，也就没有
/// 「已 drain 的句柄再消音会永久污染表」这一类陷阱。
///
/// 代价 O(n)（n = 未到点 tock 数，通常个位数），只在扑杀路径上调用（3 处）。
pub fn mute(handle: u64) {
    let mut i = TIMER_HEAP.inner.lock();
    i.heap.retain(|Reverse((_, h))| *h != handle);
    TIMER_HEAP.recompute_nearest(&i);
}

/// 最近一个到点时刻（锁外原子读；None = 无）。
pub fn due() -> Option<Instant> {
    let t = TIMER_HEAP.peek_nearest();
    (t != NONE).then_some(Instant::from_ticks(t))
}

/// 取出已到点 tock 的句柄，**写进调用方给的缓冲**，返回条数（≤ 缓冲长）。
///
/// 缓冲由调用方出，是为了让这条路径上**一处分配都不发生**：本函数只在 TIMER_HEAP
/// 锁内弹堆 + 刷镜像、锁外不碰任何东西，而调用方（`redeem`）在锁外用栈上的固定
/// 缓冲继续处理——时钟路径（S-timer / 空闲核归队）没有失败域，`Vec` 在这里就是
/// 一颗地雷。缓冲满则余下留到下一拍（尽力而为的语义与旧版一致）。
pub fn drain(now: Instant, out: &mut [u64]) -> usize {
    let mut n = 0usize;
    let mut i = TIMER_HEAP.inner.lock();
    let now = now.as_ticks();
    while let Some(Reverse((t, _))) = i.heap.peek() {
        if *t > now || n >= out.len() {
            break;
        }
        let Reverse((wake_at, handle)) = i.heap.pop().expect("peeked non-empty heap entry");
        out[n] = handle;
        n += 1;
        // 迟到读数：登记的 wake_at 到现在才被兑现（刻度）。锁内只做 Relaxed 原子，
        // 不分配、不阻塞、不 putln——本函数持 `TIMER_HEAP` 锁。
        let late = now.saturating_sub(wake_at);
        LATE_N.fetch_add(1, Ordering::Relaxed);
        LATE_SUM.fetch_add(late, Ordering::Relaxed);
        LATE_MAX.fetch_max(late, Ordering::Relaxed);
    }
    TIMER_HEAP.recompute_nearest(&i);
    n
}

/// 迟到读数 `(项数, 最迟刻度, 迟到刻度总和)`——停机读出口调一次。
///
/// 毫秒换算由调用方按 `clock::ticks_to_duration` 的 `hertz()` 现算，不写死频率。
pub fn late_stats() -> (u64, u64, u64) {
    (
        LATE_N.load(Ordering::Relaxed),
        LATE_MAX.load(Ordering::Relaxed),
        LATE_SUM.load(Ordering::Relaxed),
    )
}
