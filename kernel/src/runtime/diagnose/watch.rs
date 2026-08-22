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
use crate::runtime::chrono::clock::{self, Instant};

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
/// 哨所标准钟：全局单调时刻（判据与打点的唯一「今」来源）。仅 now() 推进；
/// 回跳/超限不推进（钳制）。初值 0，threshold 启用时上弦为启动时刻。
static WALL: AtomicU64 = AtomicU64::new(0);
/// 时钟可疑：raw 超前 WALL 超 SUSPECTTO → 置位（判据撤岗）；下次正常前向推进清位。
static SUSPECT: AtomicBool = AtomicBool::new(false);
/// 可疑上限（ticks；threshold 派生 = LIVETO×10）。
static SUSPECTTO: AtomicU64 = AtomicU64::new(0);
/// 护栏额度（ticks；threshold 派生 = LIVETO/2；wait 限睡用）。0 = 未注入。
static CAPACITY: AtomicU64 = AtomicU64::new(0);
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

/// 报时：读裸表 → 单调钳入哨所标准钟（WALL）→ 返今。判据与打点的唯一时间入口。
///
/// 语义：返回恒单调非降。回跳/持平（raw ≤ WALL）→ 今 = WALL；超前且超限
/// （raw − WALL > SUSPECTTO）→ 不推进、置 Suspect（判据撤岗），今 = WALL；
/// 正常前向 → WALL = raw 并复岗（清 Suspect）。
pub fn now() -> Instant {
    let raw = clock::now().as_ticks();
    let wall = WALL.load(Ordering::Acquire);
    if raw <= wall || raw.wrapping_sub(wall) > SUSPECTTO.load(Ordering::Relaxed) {
        if raw > wall {
            mark_suspect();
        }
        return Instant::from_ticks(wall);
    }
    match WALL.compare_exchange(wall, raw, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            SUSPECT.store(false, Ordering::Release);
            Instant::from_ticks(raw)
        }
        Err(cur) => Instant::from_ticks(cur.max(raw)),
    }
}

/// 时钟可疑点名：首次置位记事件（重复置位静默）。
fn mark_suspect() {
    if !SUSPECT.swap(true, Ordering::AcqRel) {
        crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Watch(
            crate::runtime::diagnose::trace::WatchEvent::Suspect,
        ));
    }
}

/// 打点报岗：本核完成一次确凿进展（给定今，须为 [`now`] 产物）。禁用时是
/// 廉价 relaxed 判定后跳过。
pub fn pulse(w: Instant) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let me = machine::hart_id();
    let now = w.as_ticks();
    if me < POOL {
        BEAT[me].store(now, Ordering::Relaxed);
    }
    LASTBEAT.store(now, Ordering::Relaxed);
    LASTBEATHART.store(me, Ordering::Relaxed);
}

/// 巡岗：按现况判 A/L，命中产 report（纯读，不改状态）。前置：由健康核调用
/// （自核刚 pulse 过，不误报，且今与 pulse 同源），可处 SIE=0/1 trap 上下文。
/// 时钟可疑（Suspect 置位）期间一律撤岗返回 None。
pub fn check(now: Instant, p: Probe) -> Option<WatchReport> {
    if !ENABLED.load(Ordering::Relaxed) || SUSPECT.load(Ordering::Acquire) {
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
    // B：醒着核脉搏过期 → 点名最旧那个。成立前提：所有「活着」形态都在打
    // 点——调度/trap 进度（run/wait/round 的 pulse）与内核准点（spin::lock
    // 自旋循环的节流 pulse，见 spin.rs）：BEAT 过期 = 该核既无调度/trap 进展、
    // 也无自旋等锁 = 真失速。锁相持（有人正自旋等锁）由 A 判据报告，不在此误伤。
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

/// 报警报告 → 宿主记录（feature semihosting）：`{"h","t","kind":"watch","report"}`。
/// 先于 halt 记录导出（记录序：watch 事件 → watch 记录 → halt 记录 → scene 行；
/// scene 由 panic_handler 在 alarm 之后统一 dump）。
#[cfg(feature = "semihosting")]
fn export_report(r: &WatchReport) {
    use crate::runtime::diagnose::export::{k, line, v};
    use core::fmt::Write as _;
    use table::Fmt;
    let h = machine::hart_id();
    let t = clock::now().as_ticks();
    line(|w| {
        let _ = write!(w, "\"h\":{h},\"t\":{t},\"kind\":\"watch\"");
        let _ = k(w, "report");
        let mut s = Fmt::<192>::new();
        let _ = write!(s, "{r}");
        let _ = v(w, s.as_str());
    });
}

/// 上报：拉响警报停机（trace + watch 记录，随后 panic → panic_handler 完成
/// alarm/广播/现场 dump/reset）。不返回。
///
/// 顺序纪律：**不在此处 dump 现场**——现场 dump 必须在其它核被 hunker（报警
/// 广播后）才可靠；panic_handler 的流程即「先 alarm（claim+广播）→ 打印 →
/// halt 记录 → crash_scene」。此前在 panic 前先行 dump 会在 dump 卡住时让报警
/// 永不发出、系统继续运行（多组 scene / 停不住的根因，见 23:48 现场）。
pub fn raise(r: WatchReport) -> ! {
    crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Watch(
        crate::runtime::diagnose::trace::WatchEvent::Raised,
    ));
    #[cfg(feature = "semihosting")]
    export_report(&r);
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
    WATCHED.hold_start.store(now().as_ticks(), Ordering::Relaxed);
    WATCHED.active.store(true, Ordering::Relaxed);
}

/// 撤哨：释放路径调用。先读 armed 位，未盯或非本锁即免（热路径廉价）。
pub fn unstake(addr: usize) {
    if !WATCHED.active.load(Ordering::Relaxed) || WATCHED.addr.load(Ordering::Relaxed) != addr {
        return;
    }
    WATCHED.active.store(false, Ordering::Relaxed);
}

/// 设阈值/开关（boot 注入）：注入 HOLDTO/LIVETO 并派生 SUSPECTTO（=LIVETO×10）与
/// CAPACITY（=LIVETO/2）；启用时先给 WALL 上弦（防首读超限误置 Suspect），
/// 再把基线初始化到今，避免旧 LASTBEAT=0 误报。
pub fn threshold(cfg: Threshold) {
    HOLDTO.store(
        clock::duration_to_ticks(cfg.hold_timeout),
        Ordering::Relaxed,
    );
    let live = clock::duration_to_ticks(cfg.liveness_timeout);
    LIVETO.store(live, Ordering::Relaxed);
    SUSPECTTO.store(live.saturating_mul(10), Ordering::Relaxed);
    CAPACITY.store(live / 2, Ordering::Relaxed);
    if cfg.enabled {
        WALL.store(clock::now().as_ticks(), Ordering::Relaxed);
        let w = now();
        for b in BEAT.iter() {
            b.store(w.as_ticks(), Ordering::Relaxed);
        }
        LASTBEAT.store(w.as_ticks(), Ordering::Relaxed);
    }
    ENABLED.store(cfg.enabled, Ordering::Relaxed);
}

/// 护栏额度（ticks；= LIVETO/2，threshold 派生）：wait() 以此封顶睡距——
/// 即使最近 tock 推远或镜像失真，哨兵也会周期性醒来复查（假醒不伪装进展）。
/// 0 = 阈值未注入 → 调用方回退默认睡距。
pub fn capacity() -> u64 {
    CAPACITY.load(Ordering::Relaxed)
}

/// 关键段临时撤岗。
pub fn suspend() {
    ENABLED.store(false, Ordering::Relaxed);
}
pub fn resume() {
    ENABLED.store(true, Ordering::Relaxed);
}
