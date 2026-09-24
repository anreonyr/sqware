#![no_std]
#![no_main]

//! again — **重启台**：在**同一张表、同一行**上把"起 → 停 → 放下 → 再起"走三遍。
//!
//! # 为什么要有它
//!
//! 协议 §六 写着 restart 的"重发那一半未落地"。读代码却是另一回事：`admit_start`
//! 早就允许 `State::Dead`、`Table::attach` 本来就支持"同一行换身子"、`register` 只管
//! **首启**（重名即 `Unknown`）——也就是说**机制面已经齐了，缺的只是"真走一遍"的证据**
//! （树内没有任何程序在同一行上重起过：`rig` 每轮 `Table::new()` 造新表）。
//!
//! 本台子就是那一步证据，而且是**可证伪的**：只要"重发"这条路上有任何一处把第二次
//! 判死（`admit_start` 拒、`attach` 拒、`start` 拒、`ready` 探不出来），读数就会当场
//! 变红——那时才轮到来补机制。
//!
//! ```text
//!   第 1 轮：register（首启唯一的一次）→ spawn → start → 读数
//!   每 轮：stop → watch（**落地 `Dead`**）→ Oust（父方放下旧域）→ 读数
//!   第 2/3 轮：**不再 register**，直接 spawn → start → 读数
//!
//! **照实记（本台子第一次跑出来的就是这一格）**：`stop` 只把状态推到 `Stopping`，
//! 而 `service::until` **只读不写**（它答 `Unsettled` 时"一个字都不写"）——**把
//! `Dead` 落地的是 `service::watch`**。少了这一步，`admit_start` 就按 `Stopping`
//! 把重发拒掉（实测第一版：`r=2 step=spawn REFUSED`）。所以"重发"的正确序列是
//! **stop → watch → Oust → spawn → start**，不是 stop → until。
//! ```
//!
//! # 怎么跑它
//!
//! ```text
//!   cargo image again && QEMU_ICOUNT= cargo run --release   # 与验收门同环境
//!
//! （**场景在造镜像那一刻定**——`cargo run` 只管编内核、起旁边那份 `initrd.img`；
//! 见 `crates/image`。下面那几台同理。）
//! ```
//!
//! # 读数
//!
//! 每步一行：`again: r=<轮> step=<步> state=<State> slot=<live/none> ready=<Up/Gone/Pending>`
//! 末行汇总：`again: total restarts=<成功重起的次数> failures=<被拒的次数>`
//! 判据：**`restarts=4` 且 `failures=0`**，并且末轮 `slot=live`。**4 是两段相加**：
//! 三轮回用里第 2、3 轮各一次（2 次），加收尾那段有界预算的 2 次（`budget_tries=2`）——
//! 同一个计数器记两段，故汇总行报的是 4 不是 2（旧注只写"第 2、3 轮各一次"，与打印不符）。
//!
//! # 收尾那一格：预算与放弃（协议 §六 的 Server 侧配方）
//!
//! 走满 `ROUNDS` 之后，本台子再按配方走一遍**有界预算**：预算 2 次、用尽即**放弃**，
//! 然后印出"放弃之后表里是什么"——判据是 **`state=Dead` 且 `slot=live`**（`Dead` 与
//! 坐标并存合法 ⇒ "它是什么"读得出来；"要不要再试"只有 Server 知道）。

extern crate alloc;
extern crate programs;

use programs::Reason;

use programs::supervisor::root::boot;

// 共享物住在 supervisor 目录里，由各 bin 各自声明一次（见 `needs.rs` 头注）。

use alloc::format;

use env::Name;
use programs::supervisor::system::server as service;
use protocol::system::core::{Ready, probe_ready};
use protocol::system::desk::{Announce, Slot, State, Table};
use runtime::env::debug;
use runtime::env::unit;

/// 被重起的服务（清单里已有的一个常驻程序——它起来就不走，故必须靠 `stop` 收）。
const VICTIM: &str = "churn";
/// 这一行在表里的名字（三轮回用同一个）。
const ROW: &str = "again";
/// 走几轮（1 次首启 + N-1 次重发）。
const ROUNDS: usize = 3;
/// 就绪/收尾的等待上限（毫秒，**上限族**）。
const MS: usize = 1_000;

fn main() -> Reason {
    let Some(boot) = boot::Root::take() else { return die("again: boot args unreadable") };
    let Some((elf, kind)) = find(&boot, VICTIM) else { return die("again: victim not in manifest") };
    let Ok(name) = Name::new(ROW) else { return die("again: bad row name") };

    // 一整场只用这一张表：**这就是本台子与 rig 的关键差别**（那个每轮造新表）。
    let mut table = Table::new();
    let mut restarts = 0usize;
    let mut failures = 0usize;

    for round in 1..=ROUNDS {
        // 首启唯一的一次 register；重发**不许**再 register（重名即 Unknown，见 §六）。
        if round == 1 {
            match table.register(name, Announce::None) {
                Ok(()) => say(&format!("again: r={round} step=register ok")),
                Err(_) => return die("again: register"),
            }
        } else {
            match table.register(name, Announce::None) {
                // 首启之后再登记必须被拒——这一条也是判据（拒了才说明行是复用的）。
                Err(_) => say(&format!(
                    "again: r={round} step=register refused (expected)"
                )),
                Ok(()) => {
                    failures += 1;
                    say(&format!("again: r={round} step=register ACCEPTED (bug)"));
                }
            }
        }

        // spawn：`admit_start` 在 `Dead` 上是允许的（这是"重发"的准入那一格）。
        let rep = match service::spawn(&mut table, name, elf, kind) {
            Ok(rep) => rep,
            Err(_) => {
                failures += 1;
                say(&format!("again: r={round} step=spawn REFUSED"));
                break;
            }
        };
        say(&format!("again: r={round} step=spawn ok"));

        // start（无授权、无会话、放行即起来的那一种）。
        if service::start(&mut table, name, rep, &[], None, &[], MS).is_err() {
            failures += 1;
            say(&format!("again: r={round} step=start REFUSED"));
            break;
        }
        if round > 1 {
            restarts += 1;
        }
        // 重发之后"从没起过"这个状态不该再出现：表要说出真相。
        trace(&table, name, round, "started");
        if table.find(name).map(|s| s.state) == Some(State::NeverStarted) {
            failures += 1;
            say(&format!("again: r={round} step=state NEVERSTARTED (bug)"));
        }

        // 让旧实例退场：stop（下令）→ watch（等它收干净、并把 `Dead` 落地）→ Oust（父方
        // 放下那一格）。**这里等的是 `watch` 不是 `until`**：`until` 只读，状态归 `watch` 写。
        if service::stop(&mut table, name).is_err() {
            failures += 1;
            say(&format!("again: r={round} step=stop REFUSED"));
            break;
        }
        // **落地 `Dead`**：`until` 只读，写表的是 `watch`（见头注的照实记）。
        match service::watch(&mut table, name, MS) {
            Ok(true) => {}
            _ => {
                failures += 1;
                say(&format!("again: r={round} step=watch UNSETTLED"));
            }
        }
        trace(&table, name, round, "stopped");
        // `Slot` 是"最近一次实例的坐标"、死亡不清它 ⇒ 这里读得出来，Oust 正好要用它。
        if let Some(Slot::Live { team, .. }) = table.find(name).map(|s| s.slot) {
            let _ = unit::oust(team);
        }
        trace(&table, name, round, "ousted");
    }

    // ── 收尾：有界预算 + 放弃（§六 的 Server 侧配方；预算记在**本 Server 自己手里**）──
    const BUDGET: usize = 2;
    let mut tries = 0usize;
    let mut gave_up = 0usize;
    loop {
        // `watch`：死了就把 `Dead` 落地（只读的 `until` 不算）。
        let dead = service::watch(&mut table, name, MS).unwrap_or(false);
        if !dead {
            break;
        }
        if tries >= BUDGET {
            gave_up += 1;
            break;
        }
        if let Some(Slot::Live { team, .. }) = table.find(name).map(|s| s.slot) {
            let _ = unit::oust(team);
        }
        let Ok(rep) = service::spawn(&mut table, name, elf, kind) else {
            failures += 1;
            break;
        };
        if service::start(&mut table, name, rep, &[], None, &[], MS).is_err() {
            failures += 1;
            break;
        }
        tries += 1;
        restarts += 1;
        let _ = service::stop(&mut table, name);
        let _ = service::watch(&mut table, name, MS);
    }
    // 放弃之后表里的样子：**`Dead` 与坐标并存**（这就是"它是什么"的答案）。
    trace(&table, name, ROUNDS + 1, "gave-up");

    say(&format!(
        "again: total restarts={restarts} failures={failures} budget_tries={tries} gave_up={gave_up} rows={}",
        table.rows().count()
    ));
    return 0;
}

/// 打这一步的表内事实（`state` / `slot` / `Ready` 探针）。
fn trace(table: &Table, name: Name, round: usize, step: &str) {
    let (state, slot) = match table.find(name) {
        Some(s) => (s.state, s.slot),
        None => (State::NeverStarted, Slot::None),
    };
    let slot = match slot {
        Slot::Live { .. } => "live",
        Slot::None => "none",
    };
    let ready = match probe_ready(table, name) {
        Ready::Up => "Up",
        Ready::Gone => "Gone",
        Ready::Pending => "Pending",
    };
    let state = match state {
        State::NeverStarted => "NeverStarted",
        State::Starting => "Starting",
        State::Ready => "Ready",
        State::Stopping => "Stopping",
        State::Dead => "Dead",
    };
    say(&format!(
        "again: r={round} step={step} state={state} slot={slot} ready={ready}"
    ));
}

/// 清单里按名字取镜像（只认这一条，与各台主同款）。
fn find(boot: &boot::Root, want: &str) -> Option<(&'static [u8], env::ProgramKind)> {
    let mut list = boot.programs();
    loop {
        let entry = list.next()?;
        let Ok(entry) = entry else { return None };
        if entry.name == want {
            return Some((entry.elf, entry.kind));
        }
    }
}

/// 打一行读数。台子的嘴只有调试面这一格。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

/// 起不来就报哪一句（内核收场时把这一句连同域号打出来）。
fn die(msg: &str) -> Reason {
    say(msg);
    1
}

// 本 bin 的入口那一手（`_start` 的汇编胶水 + 出口点）由构建脚本生成——见 `harness/build.rs`。
include!(concat!(env!("OUT_DIR"), "/entry_again.rs"));
