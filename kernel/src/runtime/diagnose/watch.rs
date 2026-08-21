//! watch — 值班看护：抓「停住不走」（无进展 / 锁相持）的定位工具。
//!
//! lockdep 在取锁前抓 ABBA 顺序违例；watch 抓它管不到的「活着但没在爬」——
//! 调度失速（B）/ 全系统静默（D）/ 锁长时间相持（A）。判据全读原子、无锁、
//! 不分配，可在任意 hart 的 trap/调度上下文安全执行。
//!
//! 隐喻（值班/岗哨）：pulse 打点报岗（一次确凿进展）；check 巡岗查险
//! （纯判，产 WatchReport）；raise 上报（冻结现场）；stake/unstake
//! 盯住/撤哨一个被抢的锁；threshold 设阈值开关；suspend/resume 关键段临时撤岗。
//!
//! 诚实边界：S 态在 SIE=0 自旋不收 trap，纯软件看门狗不能打断「全核关中断
//! 自旋」的 ABBA——那个场景由 lockdep 取锁前拦截、或宿主看守兜底。watch 只管
//! 「至少一个核还在节拍上」时的失速。
//!
//! 状态全静态原子（不经分配器）：崩溃现场分配器不可信。

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;

use crate::machine;
use crate::runtime::clock::{self, Instant};

/// per-hart 脉搏池上限（同 trace 纪律；超限核静默跳过）。
const POOL: usize = 64;

/// 阈值配置（boot 注入；无运行时配置 syscall）。
#[derive(Clone, Copy, Debug)]
pub struct Threshold {
    pub hold_timeout: Duration,
    pub liveness_timeout: Duration,
    pub enabled: bool,
}

/// 判据需要的外部事实（核心解耦：适配层喂入，不反向依赖 scheduler/tie/timer）。
#[derive(Clone, Copy)]
pub struct Probe {
    pub has_work: bool,
    /// per-hart WFI 睡眠位图字（位 h = hart h 正 WFI）；适配层传 tie::waiting()。
    pub asleep: &'static [AtomicUsize],
}

/// 判据产物（纯值；时间线统一 u64 ticks）。
#[derive(Clone, Copy, Debug)]
pub enum WatchReport {
    /// 某醒着核脉搏过期（B）。
    Stall { hart: usize, since: u64 },
    /// 有活却全系统无声（D）。
    WakeFailure { hart: usize, since: u64 },
    /// 锁被持超阈值且仍被等（A）。
    LockHold {
        addr: usize,
        holder: usize,
        holder_pc: usize,
        since: u64,
    },
}

impl core::fmt::Display for WatchReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WatchReport::Stall { hart, since } => write!(f, "hart {hart} stalled since {since:#x}"),
            WatchReport::WakeFailure { hart, since } => {
                write!(f, "system silent since {since:#x} (last beat hart {hart})")
            }
            WatchReport::LockHold {
                addr,
                holder,
                holder_pc,
                since,
            } => write!(
                f,
                "lock {addr:#x} held by hart {holder} pc {holder_pc:#x} since {since:#x}"
            ),
        }
    }
}

// ── 状态（原子、静态）────────────────────────────────────────────

/// per-hart 上次报岗时刻（ticks）。
static BEAT: [AtomicU64; POOL] = [const { AtomicU64::new(0) }; POOL];
/// 全局：任一核最近一次报岗时刻。
static LASTBEAT: AtomicU64 = AtomicU64::new(0);
/// 全局：最近报岗的核（诊断用）。
static LASTBEATHART: AtomicUsize = AtomicUsize::new(usize::MAX);
/// 阈值（ticks；由 threshold 注入）。
static HOLDTO: AtomicU64 = AtomicU64::new(0);
static LIVETO: AtomicU64 = AtomicU64::new(0);
/// 开关：false 时 check 一律 None。
static ENABLED: AtomicBool = AtomicBool::new(false);

/// 被盯锁槽（A 专用；单槽：只盯最新一个被抢的锁）。
struct Watched {
    active: AtomicBool,
    addr: AtomicUsize,
    holder: AtomicUsize,
    holder_pc: AtomicUsize,
    hold_start: AtomicU64,
}
static WATCHED: Watched = Watched {
    active: AtomicBool::new(false),
    addr: AtomicUsize::new(0),
    holder: AtomicUsize::new(0),
    holder_pc: AtomicUsize::new(0),
    hold_start: AtomicU64::new(0),
};

// ── 原语 ─────────────────────────────────────────────────────────

/// 打点报岗：本核完成一次确凿进展。禁用时是廉价 relaxed 判定后跳过。
pub fn pulse() {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let me = machine::hart_id();
    let now = clock::now().as_ticks();
    if me < POOL {
        BEAT[me].store(now, Ordering::Relaxed);
    }
    LASTBEAT.store(now, Ordering::Relaxed);
    LASTBEATHART.store(me, Ordering::Relaxed);
}

/// 巡岗：按现况判 A/L，命中产 report（纯读，不改状态）。前置：由健康核调用
/// （自核刚 pulse 过，不误报），可处 SIE=0/1 trap 上下文。
pub fn check(now: Instant, p: Probe) -> Option<WatchReport> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let now = now.as_ticks();
    // A：锁相持超时（槽 active = 仍有人等；释放会 unstake）。
    if WATCHED.active.load(Ordering::Relaxed)
        && now.wrapping_sub(WATCHED.hold_start.load(Ordering::Relaxed))
            > HOLDTO.load(Ordering::Relaxed)
    {
        return Some(WatchReport::LockHold {
            addr: WATCHED.addr.load(Ordering::Relaxed),
            holder: WATCHED.holder.load(Ordering::Relaxed),
            holder_pc: WATCHED.holder_pc.load(Ordering::Relaxed),
            since: WATCHED.hold_start.load(Ordering::Relaxed),
        });
    }
    if !p.has_work {
        return None;
    }
    // B：醒着核脉搏过期 → 点名最旧那个。
    let live = LIVETO.load(Ordering::Relaxed);
    let mut oldest: Option<(usize, u64)> = None;
    for (h, a) in BEAT.iter().enumerate().take(machine::hart_count()) {
        if asleep(p.asleep, h) || h >= POOL {
            continue;
        }
        let b = a.load(Ordering::Relaxed);
        if now.wrapping_sub(b) > live && oldest.is_none_or(|(_, s)| b < s) {
            oldest = Some((h, b));
        }
    }
    if let Some((hart, since)) = oldest {
        return Some(WatchReport::Stall { hart, since });
    }
    // D：有活却全系统无声。
    if now.wrapping_sub(LASTBEAT.load(Ordering::Relaxed)) > live {
        return Some(WatchReport::WakeFailure {
            hart: LASTBEATHART.load(Ordering::Relaxed),
            since: LASTBEAT.load(Ordering::Relaxed),
        });
    }
    None
}

/// 位 h 是否正 WFI 睡眠（合法深睡豁免，不判失速）。
fn asleep(words: &[AtomicUsize], h: usize) -> bool {
    let w = h / (usize::BITS as usize);
    w < words.len() && words[w].load(Ordering::Acquire) & (1 << (h % (usize::BITS as usize))) != 0
}

/// 上报：冻结现场后停机（trace + scene + panic→halt）。不返回。
pub fn raise(r: WatchReport) -> ! {
    crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Watch(
        crate::runtime::diagnose::trace::WatchEvent::Raised,
    ));
    crate::runtime::diagnose::scene::dump_crash();
    panic!("watch caught incident: {r}");
}

/// 盯住一个被抢的锁（记录持方 + 起始时刻）。幂等：同 addr 已盯则不重置 hold_start。
pub fn stake(addr: usize, holder: usize, holder_pc: usize) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if WATCHED.active.load(Ordering::Relaxed) && WATCHED.addr.load(Ordering::Relaxed) == addr {
        return;
    }
    WATCHED.addr.store(addr, Ordering::Relaxed);
    WATCHED.holder.store(holder, Ordering::Relaxed);
    WATCHED.holder_pc.store(holder_pc, Ordering::Relaxed);
    WATCHED
        .hold_start
        .store(clock::now().as_ticks(), Ordering::Relaxed);
    WATCHED.active.store(true, Ordering::Relaxed);
}

/// 撤哨：释放路径调用。先读 armed 位，未盯或非本锁即免（热路径廉价）。
pub fn unstake(addr: usize) {
    if !WATCHED.active.load(Ordering::Relaxed) || WATCHED.addr.load(Ordering::Relaxed) != addr {
        return;
    }
    WATCHED.active.store(false, Ordering::Relaxed);
}

/// 设阈值/开关（boot 注入）。启用同时把基线初始化到 now，避免旧 LASTBEAT=0 误报。
pub fn threshold(cfg: Threshold) {
    HOLDTO.store(
        clock::duration_to_ticks(cfg.hold_timeout),
        Ordering::Relaxed,
    );
    LIVETO.store(
        clock::duration_to_ticks(cfg.liveness_timeout),
        Ordering::Relaxed,
    );
    if cfg.enabled {
        let now = clock::now().as_ticks();
        for b in BEAT.iter() {
            b.store(now, Ordering::Relaxed);
        }
        LASTBEAT.store(now, Ordering::Relaxed);
    }
    ENABLED.store(cfg.enabled, Ordering::Relaxed);
}

/// 关键段临时撤岗。
pub fn suspend() {
    ENABLED.store(false, Ordering::Relaxed);
}
pub fn resume() {
    ENABLED.store(true, Ordering::Relaxed);
}

