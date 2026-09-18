#![no_std]
#![no_main]

//! root — 装配者：**按装配单起服务**，并在最后一条退场后收场。
//!
//! ```text
//! 1  启动参数 → 清单（有哪些程序）与配对块（有哪些门闩）——两块都是 boot 只读借映的
//! 2  按装配单登记整张表（`service::PLAN`）
//! 3  按装配单逐条起：建域 → 产线程 → 定会话 → 装通道 → 放行 → 等就绪 → 发门闩
//! 4  监督：**板**手里挂着每位客人的孔（封印即投信），它看出谁没了就往死亡通知那条路推
//!    一格；本域从那条路醒来 ⇒ 复核一遍（`Join{0}` 三态）⇒ 记账（`State::Dead`，坐标留着）
//!    + 放下那个死域（`Oust`）。**零轮询**：只在被叫醒时读一次
//! 5  最后一条没了 ⇒ 对仍在跑的显式 `stop`（`Ruin` = 域粒度 `Doom`）⇒ 全部记完 ⇒ 收场
//! 6  本域退出 ⇒ 级联扑杀 ⇒ 全部回收 ⇒ 自然停机（srst）
//! ```
//!
//! **本文件只剩流程**：起谁、按什么顺序、怎么发门闩、怎么算起来了，全在
//! [`service::PLAN`]（一张表）。要加第三个服务 —— 表里加一行，本文件一个字不改。
//!
//! 三件事各有其家，本文件不做：
//!
//! | 事 | 住在哪 |
//! |---|---|
//! | 起一个服务（建域/产线程/塞门闩/放行/等就绪） | `protocol::system` |
//! | 会话怎么建（交孔、认领、凑齐） | `protocol::session` |
//! | boot 的账怎么读、记录怎么编解码 | [`pairing`] |
//!
//! # 本域是设备门闩的**第一个持有者**，也是转授者
//!
//! boot 把设备树扫成门闩、连名字一起写进配对块，整块借映进本域（`platform/devices.rs`）。
//! 本域**不解释设备语义**——它只按名字挑出要交出去的那几枚。**要什么由要的人开单子**，
//! 而那张单子（[`needs`]）是**两端共用的一份**：本域不手抄名字，也不猜权与形态。
//!
//! **退出即关机**，故门的两条硬判据（自行退出 + 无 panic）照旧成立，不需要外接 timeout。
//! 常驻服务（没有"干完"这回事的那种）由本域退场时的级联收掉。

extern crate alloc;
extern crate programs;

// 共享物住在 supervisor 目录里，由两个 bin 各自声明一次（见 `needs.rs` 头注）。
#[path = "../board.rs"]
// 本域只用**板侧**那一半（客侧那三手是给服务用的）⇒ 另一半在这里是死码。
#[allow(dead_code)]
mod board;
#[path = "../needs.rs"]
mod needs;
#[path = "../pairing.rs"]
mod pairing;
#[path = "../service.rs"]
mod service;

use env::{HoleDir, Name, PieToken, TaskId};
use protocol::system::service::{Slot, State, Table};
use runtime::core::tole::Tole;
use runtime::env::mail;

use service::{E_BOOT, E_MANIFEST, E_PROGRAM, E_TABLE};

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 启动参数 → 两块账（清单 + 配对块）。读不出就没得装配。
    let Some(boot) = pairing::Root::take() else {
        service::die(E_BOOT, "root: boot args unreadable");
    };

    // 2/3. **死亡道**：一位服务一条（本域铸、记号 `gone-<名字>`；装配时各交一份给板线程
    //      —— 见 `service::start` → `board::attach`）。一服务一道 ⇒ **身份就是"哪条道响了"**：
    //      两位同时死也不会挤丢；本线程用一只**组**等任一道（`Tole`），零轮询。
    let mut lanes: [Option<PieToken>; Table::CAP] = [None; Table::CAP];
    let Ok(tole) = Tole::unseal() else {
        service::die(E_TABLE, "root: no group");
    };
    for (i, p) in service::PLAN.iter().enumerate() {
        let Ok(lane) = mail::unseal_hole(&alloc::format!("gone-{}", p.name)) else {
            continue;
        };
        let _ = tole.hang(&mail::HolePie::from_token(lane), HoleDir::Pull);
        lanes[i] = Some(lane);
    }

    // 登记整张表，再按顺序起。
    let mut table = Table::new();
    let last = match service::assemble(&mut table, &boot, &lanes) {
        Ok(last) => last,
        Err(E_MANIFEST) => service::die(E_MANIFEST, "root: manifest bad"),
        Err(E_TABLE) => service::die(E_TABLE, "root: table full"),
        Err(E_PROGRAM) => service::die(E_PROGRAM, "root: program missing"),
        Err(died) => service::die(died, "root: service failed"),
    };

    // 4/5. 监督：哪条道响 ⇒ 那一位没了 ⇒ 记账 + 放下；最后一条没了 ⇒ 显式收掉仍在跑的。
    supervise(&mut table, last, &lanes, &tole);
    // 6. **会话的收尾由会话的主人负责**：常驻线程是它起的，也是它收的。本域里那枚
    //    板线程没有 `Join` 可等（`attach` 里弃权了），故按号点名收掉——同域线程之间
    //    没有寿命耦合，内核的子域级联收不到它。**等待线程也住本域**，这一刀连它们
    //    一起收（域亡 = 成员清零）。
    board::shut();
    service::die(service::E_OK, "root: done")
}

/// 监督循环：**发现死亡 + 记账 + 放下死域**（见模块头第 4 条）。
///
/// 事件来自**板**：客人一死，它开的孔随退出钩子封印（或它自己说了退场）⇒ 板当场看出来
/// ⇒ 往**那一位的死亡道**里推一格 ⇒ 本线程从组上醒来。**一服务一道**，故"是哪一位"由
/// **哪条道响**给出——不必猜、也不会两条挤一格丢名字。
///
/// 醒来做两件事：先 `wait_gone(rep)` 等它真的收尾（板报的是"门封印了"，而 `Oust` 要的
/// 前置是"域里没有还没收尾的线程"——这一步等的是**事件**，不是节拍）；再写 `State::Dead`
/// （**不 `detach`**：坐标是"上一个实例"，留给重启与放下用）、`oust(team)` 放下那个死域、
/// 报一行。最后一条（`PLAN.last()`）没了之后，对**仍在跑的**逐个 `stop`（`Ruin` = 域粒度
/// `Doom`）——它们的死会再走同一条路回来；在册的每一行都 `Dead` 之后才收场。
fn supervise(table: &mut Table, last: Name, lanes: &[Option<PieToken>], tole: &Tole) {
    let mut stopping = false;
    loop {
        // 等任一条道响。**`Tole` 的既定用法**（板那一轮同款）：**挂起过的那一侧返回的是
        // 预置值**——内核没有第二次执行机会，故醒来必须自己按组复核，不能靠返回值拿身份。
        if tole.await_(usize::MAX).is_err() {
            // 组坏了：退回"等最后一条退场"，行为与改动前一致。
            service::wait_last(table, last);
            return;
        }
        // 复核：每条道非阻塞地问一句"有货吗"。**单槽**——道上一次死亡只响一次；一次醒来可能
        // 带走多条（两位前后脚死）。
        for (i, lane) in lanes.iter().enumerate() {
            let Some(lane) = *lane else {
                continue;
            };
            let mut one = [0u8; 1];
            if mail::HolePie::from_token(lane)
                .pull_timeout(&mut one, 0)
                .is_err()
            {
                continue; // 这一条没货
            }
            let Some(p) = service::PLAN.get(i) else {
                continue;
            };
            let Some(name) = Name::new(p.name).ok() else {
                continue;
            };
            account(table, name);
            // 最后一条走了 ⇒ 会话结束：把仍在跑的显式收掉（只下一次）。
            if name == last && !stopping {
                stopping = true;
                stop_running(table);
            }
        }
        if stopping {
            // 收场：仍在跑的已经**有界地**下过一刀并等过（见 [`stop_running`]）；等不到的那些
            // 交给本域退场时的级联——那条路是既有的可靠收场路径，不在这里等。
            return;
        }
    }
}

/// 收场那一刀：给**仍在跑的**每一位 `stop`（`Ruin` = 域粒度 `Doom`），**有界地**等它收尾并
/// 记账；等不到就报一行，交给本域退场时的级联（`board::shut` ⇒ 本域退场 ⇒ 级联扑杀）。
///
/// 为什么有界：`stop` 是"送到即回"，而实测**他杀偶发不生效**（`stop` 答 `Ok` 但目标不收尾，
/// 0–2/20 轮）——收场不能被它拖住。这一格照实记进正文。
fn stop_running(table: &mut Table) {
    for q in service::PLAN.iter() {
        let Some(name) = Name::new(q.name).ok() else {
            continue;
        };
        let running = matches!(
            table.find(name),
            Some(s) if matches!(s.state, State::Ready | State::Starting)
        );
        if !running {
            continue;
        }
        let _ = protocol::system::service::stop(table, name);
        let Some(row) = table.find(name) else {
            continue;
        };
        let Slot::Live { rep, .. } = row.slot else {
            continue;
        };
        if wait_gone(rep, STOP_MS) {
            mark_dead(table, name);
        } else {
            let _ = runtime::env::debug::put(&alloc::format!(
                "root: stuck {} （退场级联接管）",
                q.name
            ));
        }
    }
}

/// 收场那一刀的等待上限（毫秒）。**必须有界**：他杀偶发不生效，收场不能被它拖住。
const STOP_MS: usize = 300;

/// 记一位：**先等它收尾**（板报的是"门封印了"，而 `Oust` 要的前置是"域里没有还没收尾的
/// 线程"；这一步等的是事件 `Join`，不是节拍），再写 `Dead`、放下它那个域、报一行。
///
/// **幂等**：已经记过（`Dead`）就什么都不做——板报的道与我们自己杀的那一位可能都指到它。
fn account(table: &mut Table, name: Name) {
    let Some(row) = table.find(name) else {
        return;
    };
    if matches!(row.state, State::Dead) {
        return;
    }
    let Slot::Live { rep, .. } = row.slot else {
        return;
    };
    let _ = wait_gone(rep, usize::MAX);
    mark_dead(table, name);
}

/// 写 `Dead`（**不 `detach`**：坐标是"上一个实例"，留给重启与放下用）、放下那个死域、报一行。
fn mark_dead(table: &mut Table, name: Name) {
    let Some(row) = table.find(name) else {
        return;
    };
    if matches!(row.state, State::Dead) {
        return;
    }
    let Slot::Live { team, .. } = row.slot else {
        return;
    };
    table.set_state(name, State::Dead);
    let before = runtime::env::unit::heir_count().unwrap_or(0);
    let ousted = runtime::env::unit::oust(team).is_ok();
    let after = runtime::env::unit::heir_count().unwrap_or(0);
    let _ = runtime::env::debug::put(&alloc::format!(
        "root: gone {} state=Dead ousted={ousted} heir={before}→{after}",
        name.as_str()
    ));
}

/// 等它收尾：`millis` = 等待上限（`usize::MAX` = 永久）。返"收尾了吗"。
///
/// `Join` 的规范两段式：先探一次（当场就有结论），再按上限等。**问不出**（`Denied` =
/// 已入土 / 从未入册）按"收尾了"处理——与 `probe_ready` 同一条折法。
fn wait_gone(rep: TaskId, millis: usize) -> bool {
    match runtime::env::unit::join(rep, 0) {
        Ok(true) | Err(_) => return true,
        Ok(false) => {}
    }
    match runtime::env::unit::join(rep, millis) {
        Ok(true) | Err(_) => true,
        Ok(false) => false,
    }
}

/// 在册的每一行都不在跑了（`Dead`）——收场判据。
fn all_dead(table: &Table) -> bool {
    table.rows().all(|row| matches!(row.state, State::Dead))
}
