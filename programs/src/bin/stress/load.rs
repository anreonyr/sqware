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
//! workload 全都满足"总有核空闲"（`soak` 全员 1 ms 轮询睡眠、`rig` 只有一枚 `hang`）⇒
//! 这条债在树内**量不出来**。单核把那条路从机器上删掉；占核者（`busy`）负责让这唯一一颗核
//! 一刻不闲。
//!
//! **照实记**：旧注在这里引的是"release soak 16k 样本 A/B 无差别，见 `wait::block` 的
//! 照实记"——那一格**在 `wait::block` 里查不到**（那边只记了 6 打点者档与单核 3 轮），
//! 故这里不再引它。
//!
//! **环境口径（已对齐）**：下面那张表**在 `QEMU_ICOUNT=`（关掉 icount）下重取过**，
//! 与验收门同一个环境（`scripts/load.sh` 显式关掉 icount）。照实记一笔：这条债量的钟是
//! **本核的到点武装**，不是 WFI/IPI 那个被 icount 节流的钟——重取后的单核档两边读数与
//! 当初 icount 开时**同值**（97 / 99 ms）；变的只有 `traps`（645 → 643，±2 的计数噪声）。
//!
//! # 两个已经踩过的坑（都不是明显的那一个）
//!
//! 1. **`launch` 把活直接交给"挑中的那颗核"**（`kick(conductor::pick(), task)`：接进它那张
//!    队列 + 定向 IPI），核间**没有"空闲核来偷"那条路**了（`steal` 已整条删掉，见
//!    `kernel/.../scheduler/core/fetch.rs` 头注）。所以"一口气 start 12 枚"会把它们全堆在
//!    同一颗核上，别的核拿不到。⇒ 逐一放行、每次留一段空转（`GAP_US`），且**先打点者后
//!    占核者**：机器还静时放行打点者，它们才散得开。
//! 2. **台主自己不能纯空转**：S 态域任务的空转实测不吃定时器陷阱（单核整段 spin 只有 7 次
//!    陷阱、1 枚到点，其余 11 枚任务一次都没跑）。⇒ 台主自己也 `Park{1ms}`，把核让出来。
//!
//! # 判据就是内核那一行
//!
//! 停机读出口打的 `timer: late_n=… late_max_ms=… late_avg_ms=… traps=… tocks=… mutes=…`：
//! - **四处武装点**退回"写死 `beat(100 ms)`"：1 ms 的到点只能等**下一个量子拍**被兑现 ⇒
//!   `late_avg_ms` 是量子量级（几十 ms）；
//! - 四处都按 `beat_until(min(本核上限, 最近活到点))`：`late_avg_ms` ≈ 陷阱延迟。
//!
//! （**照实记**：这里原先的判据是"`wait::block` 里那次**登记后当场重武装** 关掉 / 落地后"
//! ——那一句**已按裁决删掉**（见下④），今天四处武装点里没有它，判据换成上面那两条。）
//!
//! `traps` = 定时器中断计数（"到点密 ⇒ 陷阱密"这条代价）；`tocks`/`mutes` = 登记 / 被扑杀
//! 取消的到点数（分开"打点者没跑"与"跑了又被取消"）。
//!
//! # 照实记（这台子第一次量到的）
//!
//! release、**icount 关**、`n=81`（单核档）/ 84~88（多核档），同一台子、同一颗 ELF，只差
//! **四处武装点的式子**（`trap.rs` / `hart.rs` / `stack.rs` / `fetch.rs`：`beat(写死 100 ms)`
//! → `beat_until(min(上限, 最近到点))`）与 `wait::block` 里那一次"登记后当场重武装"
//! （**本轮已按用户裁决删掉**，见下④）：
//!
//! | 档 | late_n | late_avg_ms | late_max_ms | late_max_tick | traps |
//! |---|---|---|---|---|---|
//! | 修复前 · `QEMU_SMP=1`（隔离档） | 81 | **97** | **99** | 995601 | 643 |
//! | 修复后 · `QEMU_SMP=1` | 81 | **0** | **0** | **4800**（480 µs） | 643 |
//! | 修复前 · `QEMU_SMP=4`（对照档） | 84 | 0 | 4 | 44423 | 3 |
//! | 修复后 · `QEMU_SMP=4` | 88 | 0 | **0** | 4193 | 31 |
//!
//! 四点：
//! ① **债是真的，量级正好一个失明上限**（99 ms ≈ `BLIND_MS`）——单核档两边 `late_n`/`tocks`
//!    都是 81/81，同一条命只差武装式子。
//! ② 单核档陷阱数两边一致（**643 = 643**）⇒ 到点稀疏时"到点密 ⇒ 陷阱密"这条代价并没有兑现
//!    （节拍仍由量子决定）。修后的 `late_max_tick` 逐轮抖动（四个轮次 2732~5850，即
//!    0.27~0.59 ms），表里那一格是其中一轮；量级不变。
//! ③ 多核对照档正说明"核数就是这条债的开关"：改前只剩 4 ms 的尾巴（`traps=3`），改后
//!    `late_max=0 ms`、`traps=31`——多出来的拍全花在"按最近到点武装"上（那句式子的代价面）。
//! ④ `wait::block` 那次"登记后当场重武装"（**已按裁决删掉**）：6 打点者档把它关掉，
//!    **毫秒那几格确实不变**（`late_n=281 late_avg=0 late_max=0 ms`），只有亚毫秒那格不同
//!    （关掉 744 µs / 留着 396 µs）⇒ 承重件是那三处武装点的式子，这一句是冗余的保险带。
//!    **删掉之后**（HEAD）单核隔离档 3 轮：`late_n=81 late_avg=0 late_max=0`、
//!    `late_max_tick` 2631~4689、`traps=643` ——与留着那句时同档（上表"修复后"行）。

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

/// 占核者枚数。**单核隔离档**下它把唯一那颗核钉住（一枚就够，第二枚算冗余）。
///
/// **多核档（`QEMU_SMP=4`）故意不钉满**：那一档是**对照**——只要还剩一颗空闲核，它就会
/// 按 `due()` 武装、替全局兑现，债因此被盖住（见文件头）。故这一格不写"≥ hart 数"：
/// 旧注那么写，与 `QEMU_SMP=4` 那条跑法自相矛盾。
const HOGS: usize = 2;

/// 打点者枚数：**只留一枚**——这是决定性的设计点。
///
/// 到点密的台子量不出这条债：多枚 1 ms 打点者会让"最近到点"永远存在，于是**光靠陷阱路径
/// 那一句 `beat_until` 就已经自洽**（实测 6 枚打点者档：毫秒那几格 `0/0`，亚毫秒那格只差
/// 744 µs / 396 µs——见下④）。要让"武装式子"这件事**可判**，到点必须**稀疏**：打点者两次
/// 登记之间堆是空的 ⇒ 上一次武装只能按失明上限（100 ms）⇒ 式子不对就必然晚到量子量级。
const PARKERS: usize = 1;

/// 台主自己睡多少次（每次 1 ms）。台主也是打点者之一（头注坑 2），故它的到点同样被测。
///
/// **不能开大**：满负荷下台主每 ~(任务数 × 量子) 才轮到一次，3000 次要跑几十分钟。40 次
/// ≈ 半分钟墙钟，其余样本由打点者出。
const ROUNDS: usize = 40;

/// 放行之间的空转（微秒）：让刚放行的那一枚**先跑起来**，再放下一个。
/// （旧注写的是"给每次 `kick` 留'一枚空闲核来偷'的窗口"——`steal` 已删，见坑 1。）
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
