#![no_std]
#![no_main]

//! # 怎么跑它（**核数就是这条债的开关**）
//! ```text
//!   QEMU_SMP=1 cargo image load debug && cargo run   # 机制隔离档：一核，没有第二颗核能替全局兑现
//!   QEMU_SMP=4 cargo image load debug && cargo run   # 有核空闲 ⇒ 债被"空闲核按 due() 武装"盖住
//! 为什么单核才对：`redeem`/`drain` 是**全局**的，而**任何一颗空闲核**都会按 `due()` 武装、
//! 在到点那一刻醒来兑现**全部**到点 ⇒ 只要机器上还剩一颗核空闲，旧口径也不迟到。树内
//! workload 全都满足"总有核空闲"（`soak` 全员 1 ms 轮询睡眠、`rig` 只有一枚 `hang`）⇒
//! 这条债在树内**量不出来**。单核把那条路从机器上删掉；占核者（`busy`）负责让这唯一一颗核
//! 一刻不闲。
//! **环境口径（已对齐）**：下面那张表**在 `QEMU_ICOUNT=`（关掉 icount）下重取过**，
//! 与验收同一个环境（显式 `QEMU_ICOUNT=` 关掉 icount）。这条债量的钟是
//! **本核的到点武装**，不是 WFI/IPI 那个被 icount 节流的钟——重取后的单核档两边读数与
//! icount 开时**同值**（97 / 99 ms）；变的只有 `traps`（645 → 643，±2 的计数噪声）。
//! # 两个已经踩过的坑（都不是明显的那一个）
//! 1. **`launch` 把活直接交给"挑中的那颗核"**（`kick(conductor::pick(), task)`：接进它那张
//!    队列 + 定向 IPI），核间**没有"空闲核来偷"那条路**了（`steal` 已整条删掉，见
//!    `kernel/.../scheduler/core/fetch.rs` 头注）。所以"一口气 start 12 枚"会把它们全堆在
//!    同一颗核上，别的核拿不到。⇒ 逐一放行、每次留一段空转（`GAP_US`），且**先打点者后
//!    占核者**：机器还静时放行打点者，它们才散得开。
//! 2. **台主自己不能纯空转**：S 态域任务的空转实测不吃定时器陷阱（单核整段 spin 只有 7 次
//!    陷阱、1 枚到点，其余 11 枚任务一次都没跑）。⇒ 台主自己也 `Park{1ms}`，把核让出来。
//! # 判据就是内核那一行
//! 停机读出口打的 `timer: late_n=… late_max_ms=… late_avg_ms=… traps=… tocks=… mutes=…`：
//! - **四处武装点**退回"写死 `beat(100 ms)`"：1 ms 的到点只能等**下一个量子拍**被兑现 ⇒
//!   `late_avg_ms` 是量子量级（几十 millis）；
//! - 四处都按 `beat_until(min(本核上限, 最近活到点))`：`late_avg_ms` ≈ 陷阱延迟。
//! `traps` = 定时器中断计数（"到点密 ⇒ 陷阱密"这条代价）；`tocks`/`mutes` = 登记 / 被扑杀
//! 取消的到点数（分开"打点者没跑"与"跑了又被取消"）。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;
use programs::system::control::core::unit::Declaration;
use programs::system::control::serve::task::{Image, Launch, Readiness};

use env::Wait;
use programs::Reason;

use programs::harness::tick;

use programs::boot::{Accounts, Catalog};

use core::time::Duration;

use programs::system::control::core::unit::{Announce, Table};
use programs::system::control::serve::task as service;
use programs::unit::Ending;
use protocol::debug;

/// 占核者与打点者的**清单名**（programs::unit::PROGRAMS 里 `wanted_by` 含 `load` 的那两行）
const HOG_ELF: &str = "busy";
const PARKER_ELF: &str = "park";

/// 占核者枚数。**单核隔离档**下它把唯一那颗核钉住（一枚就够，第二枚算冗余）
/// **多核档（`QEMU_SMP=4`）故意不钉满**：那一档是**对照**——只要还剩一颗空闲核，它就会
const HOGS: usize = 2;

/// 打点者枚数：**只留一枚**——这是决定性的设计点
/// 到点密的台子量不出这条债：多枚 1 ms 打点者会让"最近到点"永远存在，于是**光靠陷阱路径
/// 那一句 `beat_until` 就已经自洽**（实测 6 枚打点者档：毫秒那几格 `0/0`，亚毫秒那格只差
/// 744 µs / 396 µs——见下④）。要让"武装式子"这件事**可判**，到点必须**稀疏**：打点者两次
/// 登记之间堆是空的 ⇒ 上一次武装只能按失明上限（100 ms）⇒ 式子不对就必然晚到量子量级
const PARKERS: usize = 1;

/// 台主自己睡多少次（每次 1 ms）。台主也是打点者之一（头注坑 2），故它的到点同样被测
/// **不能开大**：满负荷下台主每 ~(任务数 × 量子) 才轮到一次，3000 次要跑几十分钟。40 次
/// ≈ 半分钟墙钟，其余样本由打点者出
const ROUNDS: usize = 40;

/// 放行之间的空转（微秒）：让刚放行的那一枚**先跑起来**，再放下一个
const GAP_US: usize = 200;

/// 表里一名一行而 `register` 重名即失败，故名字静态列死（`Table::CAP = 16`，够）
const HOG_NAMES: [&str; HOGS] = ["hog0", "hog1"];
const PARKER_NAMES: [&str; PARKERS] = ["park0"];

#[programs::entry]
fn main() -> Reason {
    let Some(accounts) = Accounts::take() else {
        return die("load: boot args unreadable");
    };
    let Some(hog) = Catalog::of_boot(&accounts).and_then(|list| list.find(HOG_ELF)) else {
        return die("load: busy not in manifest");
    };
    let Some(parker) = Catalog::of_boot(&accounts).and_then(|list| list.find(PARKER_ELF)) else {
        return die("load: park not in manifest");
    };
    let (hog, hog_kind) = (hog.elf, hog.kind);
    let (parker, parker_kind) = (parker.elf, parker.kind);

    let (iters_per_ms, ms_per_tick) = tick::calibrate();
    debug!("load: calib iters_per_ms={iters_per_ms} ms_per_tick={ms_per_tick}");
    let gap = (iters_per_ms.saturating_mul(GAP_US) / 1_000).max(1);

    let mut table = Table::new();
    let mut loader = programs::system::loader::Loader::new();
    let mut rows = 0usize;
    for name in PARKER_NAMES {
        if !spawn_one(&mut table, &mut loader, name, parker, parker_kind) {
            return die("load: spawn parker");
        }
        rows += 1;
        tick::spin_iters(gap);
    }
    for name in HOG_NAMES {
        if !spawn_one(&mut table, &mut loader, name, hog, hog_kind) {
            return die("load: spawn hog");
        }
        rows += 1;
        tick::spin_iters(gap);
    }
    debug!("load: spawned rows={rows} hogs={HOGS} parkers={PARKERS} rounds={ROUNDS}");

    // 台主自己：每 1 ms 让出一次核（**不许纯空转**，见头注坑 2）。
    let t0 = env::chrono::ticks();
    for _ in 0..ROUNDS {
        let _ = runtime::core::task::sleep(Duration::from_millis(1));
    }
    let t1 = env::chrono::ticks();
    debug!("load: ran rounds={ROUNDS} ticks={t0}→{t1}");

    for name in PARKER_NAMES.iter().chain(HOG_NAMES.iter()) {
        let _ = service::ruin(&mut table, name);
    }
    debug!("load: stopped all rows");
    return 0;
}

/// 造一行：注册名 → 造（`spawn`）→ 放行（`start`，门闩空、无会话、不认记号、不等待）
/// 任何一步失败都返回 `false`（台主自己报 `die`）
fn spawn_one(
    table: &mut Table,
    loader: &mut programs::system::loader::Loader,
    name: &'static str,
    elf: &'static [u8],
    kind: env::ProgramKind,
) -> bool {
    let name = name.to_string();
    if table
        .register(Declaration {
            name: name.clone(),
            announce: Announce::None,
            restart: Ending::Transient,
        })
        .is_err()
    {
        return false;
    }
    let Ok(task) = service::mint(
        table,
        loader,
        Image {
            name: name.as_str(),
            bytes: elf,
            kind,
        },
    ) else {
        return false;
    };
    service::embark(
        table,
        Launch {
            task,
            grants: &[],
            readiness: Readiness {
                name: name.as_str(),
                marks: &[],
                wait: Wait::POLL,
            },
        },
        &mut [],
    )
    .is_ok()
}

/// 铺不满就没得量
fn die(msg: &str) -> Reason {
    debug!("{}", msg);
    1
}
