//! IPI 自检（framework 档）— 「一记 SBI IPI 到底能不能把 WFI 里的核叫醒」。
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
//! # 只读、只在 framework 档
//!
//! 两处钩子在 `fetch::wait` 的 WFI 前后（[`wfi_entry`] / [`wfi_exit`]），全是 `Relaxed`
//! 计数；本模块只在 `--features framework` 档编进内核、只在 boot 拉起副核之后跑一次，
//! **不参与任何生产语义**。跑完打 `ipi:` 行，人工/门看它，不 panic、不改启动路径。
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

/// 目标核此刻是否正在 WFI 里（`wfi_entry` 置、`wfi_exit` 清）。
static IN_WFI: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 目标核的 WFI 返回次数。
static EXIT: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 其中"返回时 SSIP 置着"的次数。
static SSIP_EXIT: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
/// 每核自己读到的调度器地址（`current()`，tp 直达）。
static SELF_ADDR: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];

/// WFI 前置位（`fetch::wait` 调；framework 档）。
pub(crate) fn wfi_entry(hart: crate::hart::HartId) {
    if hart.get() < SLOTS {
        IN_WFI[hart.get()].store(1, Ordering::Relaxed);
        let me = crate::work::room::scheduler::core::current() as *const _ as usize;
        SELF_ADDR[hart.get()].store(me, Ordering::Relaxed);
    }
}

/// WFI 返回后记一笔（`fetch::wait` 调；framework 档）。`ssip` = 返回时 `sip.SSIP` 置否。
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

/// 定时器陷阱里每拍问一次（framework 档）：到点就跑一遍自检。跑完再把下一个采样点
/// 推后一个间隔（不在同一次里连跑），采样次数用尽即停。
pub(crate) fn tick_hook() {
    let now = time::read() as u64;
    let next = NEXT_SAMPLE.load(Ordering::Relaxed);
    if next == 0 || now < next {
        return;
    }
    if SAMPLES_LEFT.fetch_sub(1, Ordering::Relaxed) == 0 {
        NEXT_SAMPLE.store(0, Ordering::Relaxed);
        return;
    }
    crate::putln!(
        "ipi: [loaded] t_ms={} left={}",
        (now - BOOT_TIME.load(Ordering::Relaxed)) / 10_000,
        SAMPLES_LEFT.load(Ordering::Relaxed)
    );
    run("loaded");
    NEXT_SAMPLE.store(time::read() as u64 + SAMPLE_GAP, Ordering::Relaxed);
}

/// 自检本体：boot 拉起副核之后调一次（framework 档）。返回 `(测了几个核, 定向醒了几轮, 广播醒了几轮)`。
pub(crate) fn run(tag: &str) -> (usize, usize, usize) {
    let me = hart::hart_id();
    let n = hart::hart_count();
    if n < 2 {
        crate::putln!("ipi: {tag} only {n} hart — 自检需要至少两颗核，跳过");
        return (0, 0, 0);
    }
    let mut tested = 0;
    let mut d_total = 0;
    let mut b_total = 0;
    for target in (0..n.min(SLOTS)).map(crate::hart::HartId::new) {
        if target == me {
            continue;
        }
        let (d_woke, d_ssip, d_rounds) = rounds(target, false);
        let (b_woke, b_ssip, b_rounds) = rounds(target, true);
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
fn rounds(target: crate::hart::HartId, broadcast: bool) -> (usize, usize, usize) {
    let mut valid = 0;
    let mut woke = 0;
    let mut ssip = 0;
    for _ in 0..ROUNDS {
        // 等它进 WFI：等不到就跳过这一轮（不当成"没醒"）。
        if !wait_until(WAIT_TICKS, || {
            IN_WFI[target.get()].load(Ordering::Relaxed) != 0
        }) {
            continue;
        }
        let before = EXIT[target.get()].load(Ordering::Relaxed);
        let before_ssip = SSIP_EXIT[target.get()].load(Ordering::Relaxed);
        send(target, broadcast);
        if wait_until(BUDGET_TICKS, || {
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
