//! IPI 自检（debug 档）— 「一记 SBI IPI 到底能不能把 WFI 里的核叫醒」。
//!
//! # 为什么要有这一格
//!
//! rig A 的读数（见 `harness/src/rig.rs`）把"唤醒 ⇒ 上台"这一段逼到了墙角：
//! 投活 980 笔里 976 笔落在**正睡在 WFI** 的落点核上，SBI `SendIpi` **一次没返 Err**、
//! `sie.SSIE` 每次入睡都开着、`sstatus.SIE=0`，可一记门铃过去，那颗核**没有出来**——
//! 滞留那 319 笔每一笔都满足"自该核上一次被投活以来它一次都没醒过"，被杀那一刻它还在睡。
//! 零成本的补救（发完一记就等它出来、不出来再发，最多 4 记 / 每记约 20 µs）**没能改变
//! 局面**（`starved` 317 → 259），把定向单 bit 换成整字广播**更差**（`starved=328`）。
//!
//! 于是问题收敛成一个**无法再从台子上分辨**的二选一：**平台/固件侧**（IPI 没到，或到了
//! 而 `wfi` 不被它唤醒）还是**内核侧**（某条路径把 SSIP 清掉了）。本模块用最小的一对
//! 核把这一格单独量出来——刻意进 WFI、打一记、看它多数次里醒几次。
//!
//! # 只读、只在 debug 档
//!
//! 三处钩子：`fetch::wait` 的 WFI 前后（[`wfi_entry`] / [`wfi_exit`]）与**同一条空闲路**上的
//! 采样点（[`idle_hook`]），全是 `Relaxed` 计数；本模块只在 `debug_assertions` 档编进内核，
//! **不参与任何生产语义**。跑完打 `ipi:` 行，人工/门看它，不 panic、不改启动路径。
//!
//! # 照实记（负载期那次采样从定时器陷阱搬到了空闲路）
//!
//! 它原先挂在 `trap/mod.rs` 的 `SupervisorTimer` 那一支里，而那一格抢的是"**当时恰好在这颗核
//! 上的任务**"——实测：装配者起 guest 的两条握手（各 1000 ms 预算）被它吃掉，guest 报
//! `no tree link`、装配者报 `operator:hand` / `operator:claim`、`system: assemble` 当场收场，
//! 其后几条（含末条 `echo`）都不起。dev 档 root 景、同一份字节，连跑 6 次：
//!
//! ```text
//! 挂陷阱里（原样）                    折 5 / 成 6
//! 只在空闲路上跑（今天）              折 0 / 成 6
//! 挂陷阱里但把每轮预算改短到 1 ms     折 0 / 成 6，可 **early 那次读数塌了**
//!                                     （directed=0/13、2/14、1/16，原样是 7..13 / 16）
//! 挂陷阱里只治下溢                    折 4 / 成 6
//! ```
//!
//! 搬的理由就是最后那句对照：空闲路占的是"**本来就要睡的核**"，不在任何任务的预算里；
//! 而每轮预算**不能一律改短**——那会毁掉 boot 那次（`early`）的判据。
//!
//! # 照实记（这一趟现在自己有上界）
//!
//! 借空闲核不等于不花代价：它仍把那颗核占住，一颗刚被唤醒、定向投到这颗核的任务就得等它。
//! 故负载期这一趟带 [`LOADED_TICKS`]（100 ms），到点收工、把已经量到的报出来（`valid` 因此
//! 可能小于 [`ROUNDS`]——"负载期没抓够轮数"本身就是读数）。boot 那次干净上下文（`early`）
//! **不限**：那一趟的读数是本模块的判据，一字不动。
//!
//! # 照实记（计数器下溢）
//!
//! 原先的停止判据 `SAMPLES_LEFT.fetch_sub(1) == 0` 漏了跨核的序（"先读 `next`、再
//! `fetch_sub`"不是原子的）——实机量到 `left=5,4,3,2,1,0` 之后**接着**
//! `left=18446744073709551614…`：计数器翻上去，那台自检**再也不停**（green 跑里跑到 10~12 次
//! 还在涨）。今天改用 `try_update(… checked_sub)`：到 0 答 `Err`，永不下溢。
//!
//! # 判据怎么读
//!
//! 每个目标核、每种打法各 `ROUNDS` 轮。每轮：等它进 WFI → 记下它的 WFI 返回计数 →
//! 打一记 → 在 `BUDGET_TICKS` 预算内看那个计数涨不涨。
//!
//! - `directed=a/N` 与 `broadcast=b/N`：醒了几轮。**两个都接近 0** ⇒ IPI 在这台机器上
//!   叫不醒 WFI 里的核（平台/固件侧），"门铃当快路径 + 兜底睡到永远"就不成立。
//! - `ssip=`：醒来时 SSIP 是不是置着的。醒得多而 `ssip` 少 ⇒ 是定时器之类的别的源。
//! - 定向与广播**不同** ⇒ 差别在掩码/寻址那一支，而不是"IPI 本身不通"。

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use riscv::register::time;
use sbi::ecall::SArgs;
use sbi::{self, fid};

use crate::hart;

/// 每个核最多 8 个槽位（探针表；核数超过就只量前 8 个）。
const SLOTS: usize = 8;
/// 每核每种打法打几轮。
const ROUNDS: usize = 16;
/// 等目标核进 WFI 的预算（10 MHz timebase 下 100 ms = 1 000 000 tick）。
const WAIT_TICKS: u64 = 1_000_000;
/// 打出一记之后等它出来的预算（10 MHz 下 10 ms = 100 000 tick）。
const BUDGET_TICKS: u64 = 100_000;
/// **负载期那一趟的总上界**（10 MHz 下 100 ms = 1 000 000 tick）。
///
/// 负载期这次是借一颗**本该睡的核**跑的（见 [`idle_hook`]），但它仍会把那颗核占住 ⇒
/// 这一趟必须自己有上界：到点就收工、把已经量到的报出来（`valid` 可能小于 [`ROUNDS`]）。
const LOADED_TICKS: u64 = 1_000_000;

/// 目标核此刻是否正在 WFI 里（`wfi_entry` 置、`wfi_exit` 清）。
static IN_WFI: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 目标核的 WFI 返回次数。
static EXIT: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 其中"返回时 SSIP 置着"的次数。
static SSIP_EXIT: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 每核自己读到的调度器地址（`current()`，tp 直达）。
static SELF_ADDR: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

/// WFI 前置位（`fetch::wait` 调；debug 档）。
pub(crate) fn wfi_entry(hart: crate::hart::HartId) {
    if hart.get() < SLOTS {
        IN_WFI[hart.get()].store(1, Ordering::Relaxed);
        let me = crate::work::room::scheduler::core::current() as *const _ as usize;
        SELF_ADDR[hart.get()].store(me, Ordering::Relaxed);
    }
}

/// WFI 返回后记一笔（`fetch::wait` 调；debug 档）。`ssip` = 返回时 `sip.SSIP` 置否。
pub(crate) fn wfi_exit(hart: crate::hart::HartId, ssip: bool) {
    if hart.get() < SLOTS {
        IN_WFI[hart.get()].store(0, Ordering::Relaxed);
        EXIT[hart.get()].fetch_add(1, Ordering::Relaxed);
        if ssip {
            SSIP_EXIT[hart.get()].fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ── 负载期采样：同一条自检，换执行时点（E2） ──
//
// 启动期那次是**干净上下文**（没有任务、没有到点登记）；这一支把它搬到**有负载时**由
// 定时器陷阱触发（发送方 = 当时那颗核、正在跑任务）。两者只差执行时点，自检代码不变
// ⇒ 若负载期也醒，锅在 rig 那条唤醒路径；若负载期也不醒，锅在负载下的唤醒机制本身。
static BOOT_TIME: AtomicU64 = AtomicU64::new(0);
static NEXT_SAMPLE: AtomicU64 = AtomicU64::new(0);
static SAMPLES_LEFT: AtomicUsize = AtomicUsize::new(0);

/// 采样间隔（10 MHz timebase 下 5e6 = 0.5 s）与采样次数。
const SAMPLE_GAP: u64 = 5_000_000;
const SAMPLES: usize = 6;

/// 布下负载期采样点（boot 拉起副核之后调一次）。
pub(crate) fn start_delayed() {
    let now = time::read() as u64;
    BOOT_TIME.store(now, Ordering::Relaxed);
    NEXT_SAMPLE.store(now + SAMPLE_GAP, Ordering::Relaxed);
    SAMPLES_LEFT.store(SAMPLES, Ordering::Relaxed);
}

/// 负载期采样点：**空闲路上问一次**（`fetch::wait` 里 WFI 之前那一格调）——到点就跑一遍自检。
///
/// 跑到哪、为什么从陷阱搬到这里、为什么这一趟有上界、以及计数器那一格原先怎么下溢的，
/// 都写在文件头那几段照实记里。这里只说次序：**先占采样点这一格、再领名额、最后才跑**——
/// 三步都占不到就当场返回（别的核刚占走 / 名额用尽）。
pub(crate) fn idle_hook() {
    let now = time::read() as u64;
    let next = NEXT_SAMPLE.load(Ordering::Relaxed);
    if next == 0 || now < next {
        return;
    }
    // **占这一格**：把下一个采样点推后（占不到 ⇒ 别的核刚占走）。
    if NEXT_SAMPLE
        .compare_exchange(next, now + SAMPLE_GAP, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    // **领名额**：到 0 ⇒ 收工（`checked_sub` 保证**永不下溢**）。
    if SAMPLES_LEFT
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_err()
    {
        NEXT_SAMPLE.store(0, Ordering::Relaxed);
        return;
    }
    crate::putln!(
        "ipi: [loaded] t_ms={} left={}",
        (now - BOOT_TIME.load(Ordering::Relaxed)) / 10_000,
        SAMPLES_LEFT.load(Ordering::Relaxed)
    );
    run("loaded", Some(now + LOADED_TICKS));
}

/// 自检本体：boot 拉起副核之后调一次（debug 档）。返回 `(测了几个核, 定向醒了几轮, 广播醒了几轮)`。
///
/// `due`：**整趟的总上界**（timebase tick；`None` = 不限）。到点就收工、把已经量到的报出来
/// ——负载期那一趟必须有界（见 [`LOADED_TICKS`]），boot 那次干净上下文不限。
///
/// **上界按趟平分**（每个核 × 每种打法各一趟）：不平分的话第一趟能把整趟预算吃光
/// （实测：负载期那几行只剩第一个核有读数，其余全是 `0/0`）。于是每一趟都拿得到那一段
/// 预算，"负载期抓了几轮"就成了可比读数。
pub(crate) fn run(tag: &str, due: Option<u64>) -> (usize, usize, usize) {
    let due = due.unwrap_or(u64::MAX);
    let me = hart::hart_id();
    let n = hart::hart_count();
    if n < 2 {
        crate::putln!("ipi: {tag} only {n} hart — 自检需要至少两颗核，跳过");
        return (0, 0, 0);
    }
    // 还剩几趟（本核不测，故 核数 − 1，每核定向 + 广播各一趟）。
    let mut left_passes = (n.min(SLOTS) - 1) * 2;
    let mut tested = 0;
    let mut d_total = 0;
    let mut b_total = 0;
    for target in (0..n.min(SLOTS)).map(crate::hart::HartId::new) {
        if target == me {
            continue;
        }
        let d_due = share(due, left_passes);
        left_passes -= 1;
        let (d_woke, d_ssip, d_rounds) = rounds(target, false, d_due);
        let b_due = share(due, left_passes);
        left_passes -= 1;
        let (b_woke, b_ssip, b_rounds) = rounds(target, true, b_due);
        let self_addr = SELF_ADDR[target.get()].load(Ordering::Relaxed);
        let expect = crate::work::room::scheduler::core::scheduler_addr(target);
        crate::putln!(
            "ipi: {tag} target={target} directed={d_woke}/{d_rounds} ssip={d_ssip} broadcast={b_woke}/{b_rounds} ssip={b_ssip} match={}",
            self_addr == expect
        );
        tested += 1;
        d_total += d_woke;
        b_total += b_woke;
    }
    crate::putln!(
        "ipi: {tag} me={me} n={n} tested={tested} directed_woke={d_total} broadcast_woke={b_total}"
    );
    (tested, d_total, b_total)
}

/// 对 `target` 打 `ROUNDS` 轮，返回 `(醒了轮数, 其中 SSIP 置着的轮数, 有效轮数)`。
/// `broadcast` ⇒ 整字掩码（与 `yell` 同协议）；否则定向单 bit（与 `kick` 同协议）。
///
/// `due` = 整趟的总上界：**每轮之前看一次**，到点就收工；单次等待也**被剩下的预算收着**
/// （不让一次 `wait_until` 越界）。
fn rounds(target: crate::hart::HartId, broadcast: bool, due: u64) -> (usize, usize, usize) {
    let mut valid = 0;
    let mut woke = 0;
    let mut ssip = 0;
    for _ in 0..ROUNDS {
        let Some(left) = budget_left(due) else { break };
        // 等它进 WFI：等不到就跳过这一轮（不当成"没醒"）。
        if !wait_until(WAIT_TICKS.min(left), || {
            IN_WFI[target.get()].load(Ordering::Relaxed) != 0
        }) {
            continue;
        }
        let Some(left) = budget_left(due) else { break };
        let before = EXIT[target.get()].load(Ordering::Relaxed);
        let before_ssip = SSIP_EXIT[target.get()].load(Ordering::Relaxed);
        send(target, broadcast);
        if wait_until(BUDGET_TICKS.min(left), || {
            EXIT[target.get()].load(Ordering::Relaxed) > before
        }) {
            woke += 1;
            if SSIP_EXIT[target.get()].load(Ordering::Relaxed) > before_ssip {
                ssip += 1;
            }
        }
        valid += 1;
    }
    (woke, ssip, valid)
}

/// 打一记 IPI：`broadcast` 用**合法**整字掩码（只含已启动的 hart，同 `conductor::yell`
/// 的形状），否则单 bit（同 `kick`）。
///
/// **照实记**：第一版广播用的是 `usize::MAX`（含不存在的 hart 位），两种上下文里都几乎
/// 全是 `0/16` ⇒ 顺带量到一条硬事实：**掩码不合法时 SBI 返 Ok 但不投递**。"`ipi_err=0`
/// 不等于送到"这句话就是从这一格来的。
fn send(target: crate::hart::HartId, broadcast: bool) {
    let (word, bit) = target.bit();
    let n = hart::hart_count().min(usize::BITS as usize);
    let mask = if broadcast { (1usize << n) - 1 } else { bit };
    let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
        .args(SArgs {
            a0: mask,
            a1: word * (usize::BITS as usize),
            ..Default::default()
        })
        .call();
}

/// 自旋到 `pred` 为真或 `budget` 刻度用尽。`compiler_fence` 是硬要求：relaxed 原子读
/// 会被 LLVM 提出自旋环（第一版重试实验就栽在这里，读数看起来像"目标永不醒"）。
fn wait_until(budget: u64, pred: impl Fn() -> bool) -> bool {
    let deadline = time::read() as u64 + budget;
    loop {
        core::sync::atomic::compiler_fence(Ordering::Acquire);
        if pred() {
            return true;
        }
        if time::read() as u64 >= deadline {
            return false;
        }
        core::hint::spin_loop();
    }
}

/// 到 `due` 还剩多少 tick（`None` = 已经到点）。整趟的上界就是靠它收着的。
fn budget_left(due: u64) -> Option<u64> {
    let left = due.saturating_sub(time::read() as u64);
    (left > 0).then_some(left)
}

/// 这一趟能花到哪：**把剩下的预算按剩下的趟数平分**（`due` 不限时原样往下传）。
fn share(due: u64, passes: usize) -> u64 {
    if due == u64::MAX {
        return due;
    }
    let now = time::read() as u64;
    now + due.saturating_sub(now) / passes.max(1) as u64
}
