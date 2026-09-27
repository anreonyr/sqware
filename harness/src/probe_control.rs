#![no_std]
#![no_main]

//! probe-control — **control 那一面的真客人**：从树上找 `/sys/control`，问一句它的话。
//!
//! task-4 那条挂载路（`control::edge::mount`）**挂出过一块查得到、取不回的门牌**：铸入口的是
//! 一枚**一次性**边沿线程，它一收尾，持树者表里那枚入口副本就被内核的派生链级联摘掉
//! （`cull` 沿 `sire` 跨任务摘后代）。这一台的判据就是那一件事的**反面**：它**在另一个域里**，
//! 走与 principal / coalition 逐字同形的路找上门，**把门牌取回来、问一句话**。
//!
//! ```text
//!   1  与树开会话（`Session::open(sire, operator::BERTH, …)`）——**必须先装路**（见下）
//!   2  `tile(["sys", "control"])` → `token()`：名字 → 号 → **那一枚门牌**（取不回来就红）
//!   3  `control::Face::of(门牌)`：收成那一面（对端 = 门牌的开者）
//!   4  问 `state` 一个**表里没有**的名字 ⇒ 期望 `Fail::Unknown`（对面答了**一句语义码**，
//!      不是"这一趟没走到"——两件事在失败域里分得开，`Bad` 才是没走到）
//!   5  问 `state` **本台自己**（本台就是装配表里的一行、此刻活着）⇒ 期望答得出一个生命阶段
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

use env::Wait;
use programs::Report;

use env::Name;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::control as ccall;
use protocol::system::operator::client as operator;
use runtime::env::unit as utask;

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-control: /sys/control reachable";

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

    // 二、树上那一格：名字（`seek`）→ 号 → **门牌那一枚**（`find` 会把它授进本表）。
    let (Ok(dir), Ok(me)) = (Name::new(ccall::frame::DIR), Name::new(ccall::frame::NAME)) else {
        panic!("probe-control: bad name");
    };
    let plate = tree
        .tile(&[dir, me], Wait::AtMost(MS))
        .and_then(|tile| tile.token(Wait::AtMost(MS)));
    let Ok(entry) = plate else {
        panic!("probe-control: /sys/control is not on the tree");
    };

    // 三、把门牌收成那一面：对端（= 门牌的开者 = 铸入口那一枚线程）由门牌自己问出来。
    let Ok(control) = ccall::Face::of(entry) else {
        panic!("probe-control: plate refused as a control face");
    };

    // 四、**表里没有的名字** ⇒ 期望那一格语义码（`Unknown`）。这一条量的正是"这一问走到了对面、
    //     对面解开了它、并把账上的结论答了回来"——`Bad`（本端这一趟没走到）在这一格是红。
    let nobody = Name::new(NOBODY).expect("probe-control: bad probe name");
    let missing = control.service(nobody).state(Wait::AtMost(MS));
    debug!("probe-control: missing={missing:?}");

    // 五、**本台自己**：装配表里有这一行、而本台此刻活着 ⇒ 答得出一个生命阶段。
    //     （真值由装配那一趟给 `Ready`；`Starting` 也在"这一行还在、这一面看得见它"之内。）
    let mine = Name::new("probe-control").expect("probe-control: bad own name");
    let alive = control.service(mine).state(Wait::AtMost(MS));
    debug!("probe-control: self={alive:?}");

    // 六、判据：**一例一条**，名字即结论（`Bad` 那一格在两条里都是红）。
    assert!(
        matches!(missing, Err(ccall::Fail::Unknown)),
        "control 没答出「表里没这个名字」：{missing:?}（`Bad` = 这一趟没走到对面）"
    );
    assert!(
        matches!(alive, Ok(ccall::State::Ready | ccall::State::Starting)),
        "control 那一面看不见本台（装配表里的一行）：{alive:?}"
    );
    return Report::note(env::EXIT_OK, OK_NOTE);
}

