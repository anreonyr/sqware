#![no_std]
#![no_main]

//! load — **忙机台**：把"空闲核"这一格从机器上删掉，量「到点兑现」这条债的收益。
//!
//! ```text
//!   校准 → 先放行打点者、后放行占核者（逐一，中间空转一小段）
//!   → 台主自己也每 1 ms Park 一次（让出核，别霸占）
//!   → 逐名 stop → 退场（级联收干净）→ 停机行 + 内核读数
//! ```
//!
//! # 怎么跑它（**核数就是这条债的开关**）
//!
//! ```text
//!   QEMU_SMP=1 scripts/load.sh 1 --release     # 机制隔离档：一核，没有第二颗核能替全局兑现
//!   QEMU_SMP=4 scripts/load.sh 1 --release     # 有核空闲 ⇒ 债被"空闲核按 due() 武装"盖住
//! ```
//!
//! 为什么单核才对：`redeem`/`drain` 是**全局**的，而**任何一颗空闲核**都会按 `due()` 武装、
//! 在到点那一刻醒来兑现**全部**到点 ⇒ 只要机器上还剩一颗核空闲，旧口径也不迟到。树内
//! workload 全都满足"总有核空闲"（`soak` 全员 1 ms 轮询睡眠、`rig` 只有一枚 `churn`）⇒
//! 这条债在树内**量不出来**（release soak 16k 样本 A/B 无差别，见 `wait::block` 的照实记）。
//! 单核把那条路从机器上删掉；占核者（`busy`）负责让这唯一一颗核一刻不闲。
//!
//! # 两个已经踩过的坑（都不是明显的那一个）
//!
//! 1. **`launch` 只推进"放行那颗核"的队列**，再 `kick()` 唤醒**一枚** WFI 核来偷；而
//!    `advance()`（陷阱路径的轮转）在队列空时**不偷**，只有 `fetch()` 才偷。所以"一口气
//!    start 12 枚"会把它们全堆在一颗核上，另外几核各拿一枚后就不动了。⇒ 逐一放行，且
//!    **先打点者后占核者**：机器还静时放行打点者，它们才会被各核偷走散开。
//! 2. **台主自己不能纯空转**：S 态域任务的空转实测不吃定时器陷阱（单核整段 spin 只有 7 次
//!    陷阱、1 枚到点，其余 11 枚任务一次都没跑）。⇒ 台主自己也 `Park{1ms}`，把核让出来。
//!
//! # 判据就是内核那一行
//!
//! 停机读出口打的 `timer: late_n=… late_max_ms=… late_avg_ms=… traps=… tocks=… mutes=…`：
//! - 武装一式**未**落地（= `wait::block` 里那次当场重武装关掉）：1 ms 的到点只能等**下一个
//!   量子拍**被兑现 ⇒ `late_avg_ms` 是量子量级（几十 ms）；
//! - 落地后：登记者当场把本核武装到该到点 ⇒ `late_avg_ms` ≈ 陷阱延迟。
//!
//! `traps` = 定时器中断计数（"到点密 ⇒ 陷阱密"这条代价）；`tocks`/`mutes` = 登记 / 被扑杀
//! 取消的到点数（分开"打点者没跑"与"跑了又被取消"）。
//!
//! # 照实记（这台子第一次量到的）
//!
//! release、`QEMU_SMP=1`、单打点者、`n=81`，同一台子只差**三处武装点的式子**：
//!
//! | | late_n | late_avg_ms | late_max_ms | traps |
//! |---|---|---|---|---|
//! | 修复前（三处写死 100 ms + 登记不武装） | 81 | **97** | **99** | 645 |
//! | 修复后 | 81 | **0** | **0**（125 µs） | 645 |
//!
//! 三点：① 债是真的，量级正好是**一个失明上限**（100 ms）；② 陷阱数两边一致 ⇒ 到点稀疏时
//! "到点密 ⇒ 陷阱密"这条代价并没有兑现（节拍仍由量子决定）；③ 只把 `wait::block` 里那次
//! "登记后当场重武装"关掉，读数**一字不变**（`late_n=281 late_max=132 µs`，6 打点者档）
//! ⇒ 承重件是三处武装点的式子，那一句是保险带（见 `wait::block` 的照实记）。

extern crate alloc;
extern crate programs;

#[path = "../supervisor/needs.rs"]
mod needs;
#[path = "../supervisor/pairing.rs"]
mod pairing;
#[path = "tick.rs"]
mod tick;

use alloc::format;
use core::time::Duration;

use env::Name;
use protocol::system::service::{self, Announce, Table};
use runtime::env::debug;
use runtime::env::room::{self, exit_with};

/// 占核者与打点者的**清单名**（`kernel/build.rs::INITRD_BINS`）。
const HOG_ELF: &str = "busy";
const PARKER_ELF: &str = "park";

/// 占核者枚数：**不少于 hart 数**，保证任一时刻都有可跑任务 ⇒ 没有核会进空闲。
/// 单核档下它是"让这唯一一颗核一刻不闲"的那一枚；多核档下要 ≥ 核数才够（见头注）。
const HOGS: usize = 2;

/// 打点者枚数：**只留一枚**——这是决定性的设计点。
///
/// 到点密的台子量不出这条债：多枚 1 ms 打点者会让"最近到点"永远存在，于是**光靠陷阱路径
/// 那一句 `beat_until` 就已经自洽**（实测 6 枚打点者时，把 `wait::block` 那次当场重武装关掉，
/// 读数一字不变：`late_n=281 late_max=132 µs`）。要让"登记者当场重武装"这一句成为唯一证人，
/// 到点必须**稀疏**：打点者两次登记之间堆是空的 ⇒ 上一次武装只能按失明上限（100 ms）⇒
/// 关掉那一句就必然晚到量子量级。
const PARKERS: usize = 1;

/// 台主自己睡多少次（每次 1 ms）。台主也是打点者之一（头注坑 2），故它的到点同样被测。
///
/// **不能开大**：满负荷下台主每 ~(任务数 × 量子) 才轮到一次，3000 次要跑几十分钟。40 次
/// ≈ 半分钟墙钟，其余样本由打点者出。
const ROUNDS: usize = 40;

/// 放行之间的空转（微秒）：给每次 `kick` 留"一枚空闲核来偷"的窗口。
const GAP_US: usize = 200;

/// 表里一名一行而 `register` 重名即失败，故名字静态列死（`Table::CAP = 16`，够）。
const HOG_NAMES: [&str; HOGS] = ["hog0", "hog1"];
const PARKER_NAMES: [&str; PARKERS] = ["park0"];

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    let Some(boot) = pairing::Root::take() else {
        die("load: boot args unreadable")
    };
    let Some((hog, hog_kind)) = find(&boot, HOG_ELF) else {
        die("load: busy not in manifest")
    };
    let Some((parker, parker_kind)) = find(&boot, PARKER_ELF) else {
        die("load: park not in manifest")
    };

    // 校准在铺负荷**之前**：此刻机器是静的，量出来的是"空载那把尺"（只用来定放行间隔）。
    let (iters_per_ms, ms_per_tick) = tick::calibrate();
    say(&format!(
        "load: calib iters_per_ms={iters_per_ms} ms_per_tick={ms_per_tick}"
    ));
    let gap = (iters_per_ms.saturating_mul(GAP_US) / 1_000).max(1);

    let mut table = Table::new();
    let mut rows = 0usize;
    for name in PARKER_NAMES {
        if !spawn_one(&mut table, name, parker, parker_kind) {
            die("load: spawn parker")
        }
        rows += 1;
        tick::spin_iters(gap);
    }
    for name in HOG_NAMES {
        if !spawn_one(&mut table, name, hog, hog_kind) {
            die("load: spawn hog")
        }
        rows += 1;
        tick::spin_iters(gap);
    }
    say(&format!(
        "load: spawned rows={rows} hogs={HOGS} parkers={PARKERS} rounds={ROUNDS}"
    ));

    // 台主自己：每 1 ms 让出一次核（**不许纯空转**，见头注坑 2）。
    let t0 = runtime::env::chrono::ticks().unwrap_or(0);
    for _ in 0..ROUNDS {
        let _ = room::sleep(Duration::from_millis(1));
    }
    let t1 = runtime::env::chrono::ticks().unwrap_or(0);
    say(&format!("load: ran rounds={ROUNDS} ticks={t0}→{t1}"));

    for name in PARKER_NAMES.iter().chain(HOG_NAMES.iter()) {
        if let Ok(name) = Name::new(name) {
            let _ = service::stop(&mut table, name);
        }
    }
    say("load: stopped all rows");
    // 退场：本域的那些行随级联一起收干净，最后一枚任务退出时内核打停机行 + 读数。
    exit_with(0)
}

/// 造一行：注册名 → 造（`spawn`）→ 放行（`start`，门闩空、无会话、不认记号、不等待）。
/// 任何一步失败都返回 `false`（台主自己报 `die`）。
fn spawn_one(
    table: &mut Table,
    name: &'static str,
    elf: &'static [u8],
    kind: env::ProgramKind,
) -> bool {
    let Ok(name) = Name::new(name) else {
        return false;
    };
    if table.register(name, Announce::None).is_err() {
        return false;
    }
    let Ok(rep) = service::spawn(table, name, elf, kind) else {
        return false;
    };
    service::start(table, name, rep, &[], None, &[], 0).is_ok()
}

/// 清单里按名字取镜像（台主只认这两条）。
fn find(boot: &pairing::Root, want: &str) -> Option<(&'static [u8], env::ProgramKind)> {
    let mut list = boot.programs();
    loop {
        let entry = list.next()?;
        let Ok(entry) = entry else { return None };
        if entry.name == want {
            return Some((entry.elf, entry.kind));
        }
    }
}

/// 打一行读数。台主的嘴只有调试面这一格。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

/// 铺不满就没得量。
fn die(msg: &str) -> ! {
    say(msg);
    exit_with(1)
}
