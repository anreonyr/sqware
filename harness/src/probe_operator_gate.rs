#![no_std]
#![no_main]

//! probe-operator-gate — **操作面那一位持全权柄的真客人**：把七格验一遍、取回那一枚入口，
//! 再替下一位（**只有 `land` 一位、读不了树**）把两格铺在**根**底下。
//!
//! ```text
//!   1  与树开会话（`Session::open(sire, operator::BERTH, …)`）——控制面那一枚记号
//!   2  SEEK /sys/operator        ⇒ 号（它是 `mount_grants` 立出的那块 Pane）
//!   3  LIST 那一号 ＋ NAME 逐个  ⇒ 七位一个不少（与 `Grant::ALL` 对得上）
//!   4  SEEK /sys/operator/land ⇒ 号，再 FIND ⇒ **那一枚入口**（能力在树上的等价物）
//!   5  LAND /probe-op-own（`mine = true`）  ⇒ 下一位顶它时要被拒的那一格
//!   6  LAND /probe-op-free（`mine = false`）⇒ 无主那一格
//! ```
//!
//! # 为什么第 5/6 步落在**根**底下
//!
//! 下一位客人只持 `land` 一位 ⇒ 它**问不得** `list` / `seek` / `name`（那三条各是另一柄权），
//! 故它认路的坐标只能自己报得出。**根是唯一不需要号的那一格**（[`Where::Root`]：根没有号，
//! 见 `operator::frame` 那一节）——于是那两格必须落在根底下，下一位才报得出坐标。
//!
//! 这一条是**量出来的**：第一版把两格铺在 `/sys/operator/zone` 底下，而那位客人只能列号问名，
//! 于是它每一次 `list` 都被面判拒掉、当场卡死——这正是这一维在按设计生效。
//!
//! # 为什么第 1 步必须在最前
//!
//! 装配者那一步按行 `claim` 本域交出去的孔（有期限 —— `operator::bridge::attach` 的
//! `Wait::AtMost(READY_MS)`），故这一台**不能先做别的手脚再装路**（照实记见
//! `harness/src/probe_bound.rs`）。
//!
//! # 为什么它要排在整张单的**最前面**（`order: Some(6)`）
//!
//! 七位是**逐位**落上去的；而停机扳机是 `canonical`（那张单上最大 `order` 那一条）。故本台
//! 离扳机越近，可用的窗口越窄——实测把它排在 `canonical` 前两位时，它拿到那块 `Pane` 之后
//! 来不及问完那七段名字就被扑杀。「要读那七格」的客人一律排在最前，见 `canonical/program.rs`。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use env::Name;
use protocol::communication::establish;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::{Face as TreeFace, Mine, Pane};
use protocol::system::operator::{EntryId, Fail, Grant, Rule};
use runtime::env::mail;
use runtime::env::unit as utask;

/// 一趟一问的期限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// **等那一段目录长出来**的额度（毫秒；每次重试睡 [`TICK_MS`]）。
const WAIT_MS: usize = 20_000;

/// 每一次重试之间睡多久（毫秒）。
const TICK_MS: usize = 20;

/// **铺完之后还压多久**（毫秒）——见 `main` 末尾那一节（"主人还在不在场"那条轴）。
///
/// **两头顶着**：短了，下一位走到"顶那一格"时主人已经走了（那一格重新可落 ⇒ 它测得的是
/// "接手"而不是"拒"）；长了，本台自己被停机扳机扑杀、读数反倒丢了。下一位那一串只有**七八趟
/// 往返**（实测都在百毫秒内），故取刚够它走完的那一档。
const HOLD_MS: usize = 1_200;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-operator-gate: seven grants mounted";

/// 声明归本台的那一格（下一位顶它 ⇒ 该拒）。**落在根底下**，见文件头。
const OWN: &str = "probe-op-own";
/// 无主的那一格（谁都能落）。
const FREE: &str = "probe-op-free";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的，见文件头）。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: no tree link");
    };
    let tree = TreeFace::of(session);

    // 一·五、**先把那两格摆好**（摆在最前）：下一位客人与本台**并发**跑，而它读不了树
    //       （"那两格摆好了没有"它问不出来）——故本台越早铺，那一条判据越稳。
    let own = spot(&tree, OWN, "probe-gate-own", Mine::Yes);
    let free = spot(&tree, FREE, "probe-gate-free", Mine::No);

    // 二、`/sys/operator` 那块 Pane：由持树者一就位那一趟立出（`Assembly::mount_grants`）。
    let Some(operator_id) = walk(&tree, "operator") else {
        panic!("probe-operator-gate: /sys/operator is not a pane");
    };
    let operator_pane = Pane::of(&tree, operator_id);

    // 三、七位一个不少：列号 ＋ 逐个问名。
    let seen = wait_seven(&operator_pane);
    assert!(
        seen == Grant::ALL.len(),
        "/sys/operator 底下没有七位（数到的只有 {seen} 位）"
    );

    // 四、`/sys/operator/land`：**整条路**译号 → **取回那一枚入口**（`find` 把它授进本表）。
    //
    //    `Face::tile` 收的是**从根写起**的那条路（见 `client.rs` 的 `Pane::tile` 那一节）
    //    ——"名字只到 `seek` 这一格"的本义：三段一段不落。
    let (Ok(sys), Ok(op), Ok(land)) = (Name::new("sys"), Name::new("operator"), Name::new("land"))
    else {
        panic!("probe-operator-gate: bad name");
    };
    let Ok(tile) = tree.tile(&[sys, op, land], Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: /sys/operator/land is not on the tree");
    };
    let land_id = tile.id();
    let Ok(cap) = tile.token(Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: find /sys/operator/land was refused");
    };
    // 那一枚是**入口**（孔），不是随便一枚号：它答得出开者（铸它的就是编排域主线程）。
    assert!(
        establish::opened_by(cap).is_some(),
        "取回来的那一枚不是一枚入口（问不出开者）"
    );

    // 五、那两格已经在（见"一·五"）——这一行只是把读数补齐。
    debug!(
        "probe-operator-gate: land={} cap={} own={} free={}",
        land_id.get(),
        cap.get(),
        own.get(),
        free.get()
    );

    // 六、**压住那两格**：`mine = true` 那一轴的判据是"**主人还在不在场**"
    //    （`Ledger::claimable` → `vested_by`），主人一走那一格就重新可落——那正是 `probe-owner`
    //    量过的"接手"那一档。故本台铺完**不能立刻退场**：下一位客人要顶的正是"主人还活着"那一格。
    let _ = runtime::env::room::sleep(core::time::Duration::from_millis(HOLD_MS as u64));
    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// `/sys/operator` 那一格自己的号——**有界重试**：那一块由**别的域**立（本台可能比它先起）。
///
/// 三手都是 [`TreeFace`] 上现成的手：`root().tile(路)` 译号（**只译号**，不取门闩）、
/// `Tile::id()` 答号、`Tile::pane()` 判"是不是一块 Pane"。
fn walk(tree: &TreeFace, name: &str) -> Option<EntryId> {
    let Ok(sys) = Name::new("sys") else {
        return None;
    };
    let Ok(want) = Name::new(name) else {
        return None;
    };
    let root = tree.root();
    let mut left = WAIT_MS;
    loop {
        // **认得出就是认出了**：`pane` 那一问失败 ⇒ 那一格此刻还不是一块窗格 ⇒ 再等一拍。
        if let Ok(tile) = root.tile(&[sys, want], Wait::AtMost(MS)) {
            let id = tile.id();
            if tile.pane(Wait::AtMost(MS)).is_ok() {
                return Some(id);
            }
        }
        if left == 0 {
            return None;
        }
        let _ = runtime::env::room::sleep(core::time::Duration::from_millis(TICK_MS as u64));
        left = left.saturating_sub(TICK_MS);
    }
}

/// 数 `/sys/operator` 底下**认得出是那一族**的几位，**有界重试**到七位到齐或额度用尽。
///
/// 判据就是 [`Grant::name`] 那七段名字（本台不认识任何别的东西）。**它等的是"那七位到齐"**：
/// 七位是逐位落上去的，本台可能比它先起。
fn wait_seven(operator_pane: &Pane<'_>) -> usize {
    let mut left = WAIT_MS;
    loop {
        let mut seen = 0usize;
        if let Ok(listing) = operator_pane.list(Wait::AtMost(MS)) {
            for id in listing.iter() {
                if let Ok(name) = operator_pane.name(id, Wait::AtMost(MS))
                    && Grant::ALL.iter().any(|g| g.name() == name.as_str())
                {
                    seen += 1;
                }
            }
        }
        if seen == Grant::ALL.len() || left == 0 {
            return seen;
        }
        let _ = runtime::env::room::sleep(core::time::Duration::from_millis(TICK_MS as u64));
        left = left.saturating_sub(TICK_MS);
    }
}

/// 在**根**底下落一格（记号只为本台这台测具而立，不进任何一族的表）。
fn spot(tree: &TreeFace, name: &str, mark: &'static str, mine: Mine) -> EntryId {
    let Ok(spot) = Name::new(name) else {
        panic!("probe-operator-gate: bad spot name");
    };
    let Ok(entry) = mail::unseal_hole(env::Mark::of(mark)) else {
        panic!("probe-operator-gate: no entry");
    };
    // **`Unknown` 重试**（与 `client.rs` 的 `road_to_id` 同一条口径）：那一格由本台与下一位
    // 客人**并发**动，而这一手是"一问一动"——`Unknown` 在这条路上说的是"这一趟没走到"，
    // 不是"这一格不许"。除它以外的失败都是确定的下一步（当场塌）。
    let mut left = WAIT_MS;
    loop {
        match tree
            .root()
            .bind(spot, entry, Rule::Public, mine, Wait::AtMost(MS))
        {
            Ok(id) => return id.id(),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(TICK_MS as u64));
                left = left.saturating_sub(TICK_MS);
            }
            Err(fail) => panic!("probe-operator-gate: land {name} failed: {fail:?}"),
        }
    }
}
