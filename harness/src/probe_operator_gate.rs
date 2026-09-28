#![no_std]
#![no_main]

//! probe-operator-gate — **操作面那一位持全权柄的真客人**：把七格验一遍、取回那一枚入口，
//! 再替下一位（**只有 `land` 一位、读不了树**）把两格铺在**根**底下。
//!
//! ```text
//!   1  与树开会话（`Session::open(sire, operator::BERTH, …)`）——控制面那一枚记号
//!   2  SEEK /svc/operator        ⇒ 号（它是 `mount_grants` 立出的那块 Pane）
//!   3  LIST 那一号一次        ⇒ 数得出 `Grant::ALL.len()` 格（名字那七问归树自己那七行读数）
//!   4  SEEK /svc/operator/land ⇒ 号，再 FIND ⇒ **那一枚入口**（能力在树上的等价物）
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
//! 这一条是**量出来的**：第一版把两格铺在 `/svc/operator/zone` 底下，而那位客人只能列号问名，
//! 于是它每一次 `list` 都被面判拒掉、当场卡死——这正是这一维在按设计生效。
//!
//! # 为什么第 1 步必须在最前
//!
//! 装配者那一步按行 `claim` 本域交出去的孔（有期限 —— `operator::bridge::attach` 的
//! `Wait::AtMost(READY_MS)`），故这一台**不能先做别的手脚再装路**（照实记见
//! `harness/src/probe_bound.rs`）。
//!
//! # 为什么它排在整张单的**最前**（`order: Some(3)`）
//!
//! 停机扳机是 `canonical`（那张单上最大 `order` 那一条），而本台问的是**树**（不需要任何驱动）
//! ⇒ 排在身份服务之后、三台驱动之前。**但"窗口"这件事不能靠排队次治**：实测本台自己会走到那两处
//! 重试额度的尽头（旧版是 20 s ＋ 20 s），于是"扳机早于它走完"就变成丢读数——**绿也没有、红
//! 也没有**（喂了输入的 39 跑里丢 20 次；完全不喂的跑里也丢过）。故额度收小到 [`WAIT_MS`]、
//! 那七段名字的读数交还给树自己（见 [`count_under`]）：数不到就**当场红**。「要读树」的客人
//! 一律排在最前，见 `canonical/program.rs`。

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
use protocol::system::operator::{EntryId, Fail, Grant, Permit};
use runtime::env::mail;
use runtime::env::unit as utask;

/// 一趟一问的期限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// **等那一段目录长出来 / 那几格到齐**的额度（毫秒；每次重试睡 [`TICK_MS`]）。
///
/// **照实记（20 s → 3 s）**：本台是**铺场者**，而它"走不完"的代价不是红——停机扳机一来就把它
/// **扑杀**（`ousted=true`、一行不打），那条读数于是**既不绿也不红**（第三种结局）。旧版给的是
/// 20 s，而这样的额度本台有**两处**（[`walk`] 与 [`count_under`]），走满就是几十秒的窗口。
/// 收紧到 3 s 之后：健康那一档（实测走完全程、含 [`HOLD_MS`]，只要 1~3 s）毫发无伤，
/// 而"数不到"那一档**当场红**（到点返回、由调用方那句 `assert` 落地）。
///
/// 这个数不是猜的：七行 `system: grant mounted at /svc/operator/{…}` 在 **t<500 ms** 就打完
/// （同一份镜像、直接起 QEMU 量过），故一个控制面会话看得见它们的时间以毫秒计。
const WAIT_MS: usize = 3_000;

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

    // 二、`/svc/operator` 那块 Pane：由持树者一就位那一趟立出（`Assembly::mount_grants`）。
    let Some(operator_id) = walk(&tree, "operator") else {
        panic!("probe-operator-gate: /svc/operator is not a pane");
    };
    let operator_pane = Pane::of(&tree, operator_id);

    // 三、那几格到齐：**数一次就够**（名字那七问归树自己那七行读数，见 [`count_under`]）。
    let seen = count_under(&operator_pane);
    assert!(
        seen == Grant::ALL.len(),
        "/svc/operator 底下没有七格（数到的只有 {seen} 格）"
    );

    // 四、`/svc/operator/land`：**整条路**译号 → **取回那一枚入口**（`find` 把它授进本表）。
    //
    //    `Face::tile` 收的是**从根写起**的那条路（见 `client.rs` 的 `Pane::tile` 那一节）
    //    ——"名字只到 `seek` 这一格"的本义：三段一段不落。
    let (Ok(sys), Ok(op), Ok(land)) = (Name::new(protocol::system::SVC), Name::new("operator"), Name::new("land"))
    else {
        panic!("probe-operator-gate: bad name");
    };
    let Ok(tile) = tree.tile(&[sys, op, land], Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: /svc/operator/land is not on the tree");
    };
    let land_id = tile.id();
    let Ok(cap) = tile.token(Wait::AtMost(MS)) else {
        panic!("probe-operator-gate: find /svc/operator/land was refused");
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
    //    （`Operator::claimable` → `vested_by`），主人一走那一格就重新可落——那正是 `probe-owner`
    //    量过的"接手"那一档。故本台铺完**不能立刻退场**：下一位客人要顶的正是"主人还活着"那一格。
    let _ = runtime::env::room::sleep(core::time::Duration::from_millis(HOLD_MS as u64));
    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// `/svc/operator` 那一格自己的号——**有界重试**：那一块由**别的域**立（本台可能比它先起）。
///
/// 三手都是 [`TreeFace`] 上现成的手：`root().tile(路)` 译号（**只译号**，不取门闩）、
/// `Tile::id()` 答号、`Tile::pane()` 判"是不是一块 Pane"。
fn walk(tree: &TreeFace, name: &str) -> Option<EntryId> {
    let Ok(sys) = Name::new(protocol::system::SVC) else {
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

/// 数 `/svc/operator` 底下**那几格到齐没有**——**一问**（`list`）＋ 有界重试。
///
/// **照实记（为什么不再逐个问名）**：那七段名字的读数归**树自己**——`mount_grants` 每落一位就抬
/// 一行 `system: grant mounted at /svc/operator/{…}`（七行，t<500 ms 打完）。本台再 `list`
/// ＋ 七次 `name` 是八趟往返，而那条路每一趟都可能**等在门外**（`client.rs::call` 那一推是
/// `Send(.., Wait::Forever)`：孔是单槽，对面没取走就永远等）⇒ 越少问越不容易挂在那儿。
///
/// **照实记（它第一跑就撞上一个真缺陷）**：改成"数格子"之后，实机读数 `seen=8`——`/svc/operator`
/// 底下是**八格**：七位 ＋ 一格**也叫 `operator` 的自己**（那段目录铸的那枚孔就挂在那儿）。
/// 旧写法**按名字数**，多出来的一格不进账，故这个缺陷一直没显形；而新写法数不满就重试到额度
/// 尽头 ⇒ 本台**整趟卡死**、被扳机扑杀（正是上面那条"既不绿也不红"）。根因与那一刀见
/// `programs/src/system/mod.rs::Assembly::mount_control` 的照实记（目录不再铸孔、不再落叶子；
/// 四份 `mount.rs` 已随回炉收成一处，故照实记归了那一格）。
/// **这一条读数因此比旧写法更硬**：它数的是"那一块窗格里真有七格"，不是"认得出七段名字"。
///
/// 那一格是**逐位**落上去的（目录先立、七位一位一位落），故"数不满"那一刻是**预期之内**的
/// ——重试到 [`WAIT_MS`] 为止；到点仍不齐就把数到的几格交回给调用方，由它 `assert` 当场红。
fn count_under(pane: &Pane<'_>) -> usize {
    let mut left = WAIT_MS;
    loop {
        let mut seen = 0usize;
        match pane.list(Wait::AtMost(MS)) {
            Ok(listing) => {
                seen = listing.iter().count();
                if seen == Grant::ALL.len() {
                    return seen;
                }
            }
            // 一问没走到（对面这趟没答）⇒ 还留着额度就再来一拍；这**不是**"那一格不在"
            // （那一格不在会答 `NotAPane`，落在同一个 `Err` 里也无妨：走到额度尽头就由上层
            // 那句 `assert` 当场红）。
            Err(_) => {}
        }
        if left == 0 {
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
            .bind(spot, entry, Permit::Unset, mine, Wait::AtMost(MS))
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
