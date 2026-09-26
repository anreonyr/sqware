//! system::supervise — **监督相**：哪一位没了、怎么记账、怎么收场（[`super`] 头注第 5 / 6 条）
//!
//! 本文件只管**起完之后一直看**：板把"某位的门封印了"变成它那条死亡道上的一格，本线程从
//! 组上醒来、按道上的名字认人、等它真收尾、写 `Dead`、放下它的域、报一行读数；名册最后一条
//! 走了之后，把仍在跑的**有界地**收掉，然后收场。
//!
//! **装配那一半不在这里**（[`super::server`]）：建域 / 放行 / 等就绪 / 收一枚都是**装配期**的
//! 事。两半之间只有两处来往——表里那几格状态（`State` / `Slot`），与那两枚原语 [`stop`] /
//! [`until`]（本文件只读它们，不重写）。
//!
//! **死亡道**（[`Lane`]）也住在这一半：那枚孔是装配期铸的（`main` 铸、`server::start` 转授给
//! 板），但它的**读者只有本文件**——一条道响一次、响完就完，正是"监督相"这件事本身。

use env::Name;
use env::Wait;
use runtime::core::pile::Pile;
use runtime::env::mail::HolePie;
use runtime::env::unit as utask;

use protocol::debug;
use protocol::system::core::Reaped;
use protocol::system::desk::{Slot, State, Table};

// 表那一侧的两手（本文件只读、不重写）与装配期铸的那一条道。
use super::server::{stop, until};
use crate::service::Lane;

/// 监督循环：**发现死亡 + 记账 + 放下死域**。
///
/// 事件来自**板**：客人一死，它开的孔随退出钩子封印（或它自己说了退场）⇒ 板当场看出来
/// ⇒ 往**那一位的死亡道**里推一格 ⇒ 本线程从组上醒来。**一服务一道**，故"是哪一位"由
/// **哪条道响**给出——不必猜、也不会两条挤一格丢名字。
///
/// 醒来做两件事：先 `service::until` 等它真的收尾（板报的是"门封印了"，而 `Oust` 要的
/// 前置是"域里没有还没收尾的线程"——这一步等的是**事件**，不是节拍）；再写 `State::Dead`
/// （**不 `detach`**：坐标是"上一个实例"，留给重启与放下用）、`oust(team)` 放下那个死域、
/// 报一行。最后一条（单子最后一条）没了之后，对**仍在跑的**逐个 `stop`（`doom` = 域粒度
/// `Doom`）——它们的死会再走同一条路回来；在册的每一行都 `Dead` 之后才收场。
pub fn run(table: &mut Table, last: Name, lanes: &[Lane], pile: &Pile) {
    // 死亡道那一格：**一页**——与门那一侧同一条规则（谁能往里推，缓冲就按**载体**的界备，
    // 不按"这条路上平常走几个字节"备）。一枚更长的推落进道里时，1 字节的读法取不出也丢不掉，
    // 那一位的死就永远记不上账。备不下 ⇒ 报一句就交给退场时的级联，不在这里赌。
    let mut lane_buf: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    if lane_buf.try_reserve_exact(runtime::PAGE_SIZE).is_err() {
        debug!("system: no room");
        return;
    }
    lane_buf.resize(runtime::PAGE_SIZE, 0);
    let mut stopping = false;
    loop {
        // 等任一条道响。**`Pile` 的既定用法**（板那一轮同款）：**挂起过的那一侧返回的是
        // 预置值**——内核没有第二次执行机会，故醒来必须自己按组复核，不能靠返回值拿身份。
        if pile.await_(Wait::Forever).is_err() {
            // 组坏了：退回"等最后一条退场"，行为与改动前一致。
            crate::service::wait_last(table, last);
            return;
        }
        // 复核：每条道非阻塞地问一句"有货吗"。**单槽**——道上一次死亡只响一次；一次醒来
        // 可能带走多条（两位前后脚死）。
        for lane in lanes {
            let Some(road) = lane.road else {
                continue;
            };
            if HolePie::from_token(road)
                .pull_timeout(&mut lane_buf, Wait::POLL)
                .is_err()
            {
                continue; // 这一条没货
            }
            let Ok(name) = Name::new(lane.name) else {
                continue;
            };
            account(table, name);
            // 最后一条走了 ⇒ 会话结束：把仍在跑的显式收掉（只下一次）。
            if name == last && !stopping {
                stopping = true;
                stop_running(table, lanes);
            }
        }
        if stopping {
            // 收场：仍在跑的已经**有界地**下过一刀并等过（见 [`stop_running`]）；等不到的
            // 那些交给本域退场时的级联——那条路是既有的可靠收场路径，不在这里等。
            return;
        }
    }
}

/// 收场那一刀：给**仍在跑的**每一位 `stop`（`doom` = 域粒度 `Doom`），**有界地**等它
/// 收尾并记账；等不到就报一行，交给本域退场时的级联。
///
/// 为什么有界：`stop` 是"送到即回"（`kill` 的口径），收场不能被一个收不掉的域拖住。
pub fn stop_running(table: &mut Table, lanes: &[Lane]) {
    for lane in lanes {
        let Some(name) = Name::new(lane.name).ok() else {
            continue;
        };
        // **本域那一枚不在这里收**：它的"域"就是本域，收它就是扑杀本域自己。它随本域退场
        // 时的"域亡＝成员清零"一起走。
        if matches!(
            table.find(name).map(|s| s.slot),
            Some(Slot::Live { team: None, .. })
        ) {
            continue;
        }
        let running = matches!(
            table.find(name),
            Some(s) if matches!(s.state, State::Ready | State::Starting)
        );
        if !running {
            continue;
        }
        let _ = stop(table, name);
        // 表里没有可等的坐标（`stop` 也答了 `Unknown`）：没得等，也不算"卡住"。
        if !matches!(table.find(name).map(|s| s.slot), Some(Slot::Live { .. })) {
            continue;
        }
        match until(table, name, Wait::AtMost(STOP_MS)) {
            Ok(Reaped::Now) => mark_dead(table, name, Reaped::Now),
            Ok(Reaped::Waited) => mark_dead(table, name, Reaped::Waited),
            // 有界期内没等出来：照实报，交出这一位。**不是"没收到"**——判决只认非阻塞
            // 那一问，这里说的是"还没收干净"。
            Ok(Reaped::Unsettled) | Err(_) => {
                debug!(
                    "system: stuck {} （退场级联接管）",
                    lane.name
                );
            }
        }
    }
}

/// 收场那一刀的等待上限（毫秒）。**必须有界**：收场不能被一个收不掉的域拖住。
const STOP_MS: usize = 300;

/// 记一位：**先等它收尾**（板报的是"门封印了"，而 `Oust` 要的前置是"域里没有还没收尾的
/// 线程"，故这一步等的是收尾事件，不是节拍），再写 `Dead`、放下它那个域、报一行。
///
/// **幂等**：已经记过（`Dead`）就什么都不做——板报的道与我们自己杀的那一位可能都指到它。
fn account(table: &mut Table, name: Name) {
    let Some(row) = table.find(name) else {
        return;
    };
    if matches!(row.state, State::Dead) {
        return;
    }
    let Slot::Live { .. } = row.slot else {
        return;
    };
    let reaped = until(table, name, Wait::Forever).unwrap_or(Reaped::Unsettled);
    mark_dead(table, name, reaped);
}

/// 写 `Dead`（**不 `detach`**：坐标是"上一个实例"，留给重启与放下用）、放下那个死域、报一行。
///
/// `reaped` = 这一位的收尾判决**及它的来路**。读数里那一格是给验收用的：`wait=now` 说明收尾
/// 早在问之前就完了，`wait=waited` 说明这一次是**等到**的；`wait=unsettled` 则是"没被确认
/// 收尾"，那时 `ousted=false` 会一起把真相摆出来。
fn mark_dead(table: &mut Table, name: Name, reaped: Reaped) {
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
    let before = utask::heir_count();
    // **本域那一枚没有别人的域可放下**（`team = None`）：放下它就是扑杀本域自己。
    let ousted = match team {
        Some(team) => utask::oust(team).is_ok(),
        None => false,
    };
    let after = utask::heir_count();
    let wait = match reaped {
        Reaped::Now => "now",
        Reaped::Waited => "waited",
        Reaped::Unsettled => "unsettled",
    };
    debug!(
        "system: gone {} state=Dead ousted={ousted} heir={before}→{after} wait={wait}{}",
        name.as_str(),
        if team.is_none() { " inner" } else { "" }
    );
}
