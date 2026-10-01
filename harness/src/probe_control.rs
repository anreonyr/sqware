#![no_std]
#![no_main]

//! probe-control — **control 那一族的真客人**：从树上找 **`/svc/sys/control/state`**（问面），问一句它的话；
//! 另取 `mint` / `start` / `stop` 三面（各带规矩）期望被拒，并拿问面发写、期望判面拒。
//!
//! task-4 那条挂载路（`control::edge::mount`）**挂出过一块查得到、取不回的门牌**：铸入口的是
//! 一枚**一次性**边沿线程，它一收尾，持树者表里那枚入口副本就被内核的派生链级联摘掉
//! （`cull` 沿 `sire` 跨任务摘后代）。这一台的判据就是那一件事的**反面**：它**在另一个域里**，
//! 走与 principal / coalition 逐字同形的路找上门，**把门牌取回来、问一句话**。
//!
//! ```text
//!   1  与树开会话（`Session::open(sire, operator::BERTH, …)`）——**必须先装路**（见下）
//!   2  `tile(["sys", "control", "state"])` → `token()`：名字 → 号 → **问面那一枚门牌**
//!      （取不回来就红；开面这一刀之后 `/svc/sys/control` 自己是一段前缀，不是一格）
//!   3  `control::Face::of(门牌)`：收成那一面（对端 = 门牌的开者）
//!   4  问 `state` 一个**表里没有**的名字 ⇒ 期望 `Fail::Unknown`（对面答了**一句语义码**，
//!      不是"这一趟没走到"——两件事在失败域里分得开，`Bad` 才是没走到）
//!   5  问 `state` **本台自己**（本台就是装配表里的一行、此刻活着）⇒ 期望答得出一个生命阶段
//!   6  取 `mint` / `start` / `stop` **三格** ⇒ 期望各答 `Denied`——那三格**带了规矩**
//!      （"许给开着这一格的那位"＝铸那几枚入口的编排域主线程）⇒ 别的域取不回
//!   7  拿**问面**那一枚入口发 `mint` / `start` / `stop` ⇒ 期望各答 `Denied`——**判面**那一句：
//!      **读面公开 ≠ 读面能写**
//! ```
//!
//! # 为什么第 1 步必须在最前
//!
//! 装配者那一步按行 `claim` 本域交出去的孔（有期限 —— `operator::bridge::attach` 的
//! `Wait::AtMost(READY_MS)`），故这一台**不能先做别的手脚再装路**：第一版把装路排在后面，
//! 装配那一侧当场报 `operator:claim`（照实记见 `harness/src/probe_bound.rs`）。
//!
//! # 为什么第 4 / 5 步要等（本台比挂载先起）
//!
//! 本台排在 `canonical` 之前（`order: Some(18)`），而 `control` 那一面是在**整表起完之后**
//! 才挂上树的（`Assembly::supervise` 那一步——挂它的是编排域主线程，它此后就进监督那一趟，
//! **这就是"铸入口那一枚必须长命"**）。故第 2 步那一问**等在门外**：门牌的号一开始还没有，
//! `Face::tile` 按额度重试（`RETRY_MS` 一拍问一次），持树者那边把它铺好了就答。
//!
//! # 判据为什么必须 **panic**
//!
//! 整机那一格判的是"有没有 `EXIT_PANIC`"（`kernel/src/work/room/conductor.rs` 的判据只此
//! 一处）：返回一个非零的 `Report` **不算红**。故这一台每一步失败都当场塌。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;

use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::Fail as TreeFail;
use protocol::service::operator::client as operator;
use protocol::system::control as ccall;
use runtime::env::unit as utask;

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-control: ask open, three faces denied";

/// **一个一定不在装配表里的名字**：第 4 步那一问的荷载。
///
/// 取"表里没有"是**故意**的：那一问要的就是"对面答得出一句语义码"。若拿一个真名字去问，
/// 答案会跟着那一台的生命阶段变（同一句话量出两件事）。
const NOBODY: &str = "probe-control-nobody";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的，见文件头）。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-control: no tree link");
    };
    let tree = operator::Face::from(&session);

    // 二、树上那**一格**：名字（`seek`）→ 号 → **门牌那一枚**（`find` 会把它授进本表）。
    //
    // **开面那一刀之后这一格是问面**（`/svc/sys/control/state`）：`/svc/sys/control` 自己成了
    // 那段前缀（一块 `Pane`，没有门牌可授）。
    // 路是**本族那一族的常量**（`/svc/sys/control`）接上那一面的名——一处都不自己拼。
    let road = ccall::DIR
        .try_join(ccall::Grant::State.name())
        .expect("probe-control: bad name");
    let plate = tree
        .tile(&road, Wait::AtMost(MS))
        .and_then(|tile| tile.token(Wait::AtMost(MS)));
    let Ok(entry) = plate else {
        panic!("probe-control: /svc/sys/control/state is not on the tree");
    };

    // 三、把门牌收成那一面：对端（= 门牌的开者 = 铸入口那一枚线程）由门牌自己问出来。
    let Ok(control) = ccall::Face::of(entry) else {
        panic!("probe-control: plate refused as a control face");
    };

    // 四、**表里没有的名字** ⇒ 期望那一格语义码（`Unknown`）。这一条量的正是"这一问走到了对面、
    //     对面解开了它、并把账上的结论答了回来"——`Bad`（本端这一趟没走到）在这一格是红。
    let nobody = NOBODY.to_string();
    let missing = control.service(nobody.clone()).state(Wait::AtMost(MS));
    debug!("probe-control: missing={missing:?}");

    // 五、**本台自己**：装配表里有这一行、而本台此刻活着 ⇒ 答得出一个生命阶段。
    //     （真值由装配那一趟给 `Ready`；`Starting` 也在"这一行还在、这一面看得见它"之内。）
    let mine = "probe-control".to_string();
    let alive = control.service(mine).state(Wait::AtMost(MS));
    debug!("probe-control: self={alive:?}");

    // 六、**带规矩那三面**：`mint` / `start` / `stop` 各取一遍 ⇒ 期望 `Denied`（树那一层）。
    //     它们带的是"许给开着这一格的那位"——而那几枚入口是**编排域主线程**铸的 ⇒ 本台取不回。
    let mut denied_cells = [false; 3];
    for (i, grant) in [ccall::Grant::Mint, ccall::Grant::Start, ccall::Grant::Stop]
        .into_iter()
        .enumerate()
    {
        let road = ccall::DIR
            .try_join(grant.name())
            .expect("probe-control: bad face name");
        let got = tree
            .tile(&road, Wait::AtMost(MS))
            .and_then(|tile| tile.token(Wait::AtMost(MS)));
        debug!("probe-control: {}=err:{got:?}", grant.name());
        denied_cells[i] = matches!(got, Err(TreeFail::Denied));
    }

    // 七、**判面**：本台手里只有**问面**那一枚入口——发写要各答 `Denied`（不是 `Bad`：那一趟
    //     走到了对面，是对面**说得清清楚楚**地拒的）。
    let write_mint = control.mint(nobody.clone(), Wait::AtMost(MS)).err();
    let write_start = control
        .service(nobody.clone())
        .start(Wait::AtMost(MS))
        .err();
    let write_stop = control.service(nobody).stop(Wait::AtMost(MS)).err();
    debug!(
        "probe-control: mint(ask)={write_mint:?} start(ask)={write_start:?} stop(ask)={write_stop:?}"
    );

    // 八、判据：**一例一条**，名字即结论（`Bad` 那一格在每一条里都是红）。
    assert!(
        matches!(missing, Err(ccall::Fail::Unknown)),
        "control 没答出「表里没这个名字」：{missing:?}（`Bad` = 这一趟没走到对面）"
    );
    assert!(
        matches!(alive, Ok(ccall::State::Ready | ccall::State::Starting)),
        "control 那一面看不见本台（装配表里的一行）：{alive:?}"
    );
    for (i, grant) in [ccall::Grant::Mint, ccall::Grant::Start, ccall::Grant::Stop]
        .into_iter()
        .enumerate()
    {
        assert!(
            denied_cells[i],
            "{} 那一格带了规矩（许给开着这一格的那位），本台该取不回",
            grant.name()
        );
    }
    assert!(
        matches!(write_mint, Some(ccall::Fail::Denied)),
        "问面发不出 Mint：{write_mint:?}（`Bad` = 这一趟没走到对面）"
    );
    assert!(
        matches!(write_start, Some(ccall::Fail::Denied)),
        "问面发不出 Start：{write_start:?}"
    );
    assert!(
        matches!(write_stop, Some(ccall::Fail::Denied)),
        "问面发不出 Stop：{write_stop:?}"
    );
    return Report::note(env::EXIT_OK, OK_NOTE);
}
