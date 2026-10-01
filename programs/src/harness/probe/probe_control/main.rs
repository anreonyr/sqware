#![no_std]
#![no_main]

//! probe-control — control 那一族的真客人：从树上找 /svc/sys/control/state（问面），问一句它的话；
//! 另取 `mint` / `start` / `stop` 三面（各带规矩）期望被拒，并拿问面发写、期望判面拒。
//! task-4 那条挂载路（control::edge::mount）**挂出过一块查得到、取不回的门牌**：铸入口的是
//! 一枚**一次性**边沿线程，它一收尾，持树者表里那枚入口副本就被内核的派生链级联摘掉
//! （`cull` 沿 `sire` 跨任务摘后代）。这一台的判据就是那一件事的**反面**：它**在另一个域里**，
//! 走与 principal / coalition 逐字同形的路找上门，**把门牌取回来、问一句话**。
//! # 为什么第 1 步必须在最前
//! # 为什么第 4 / 5 步要等（本台比挂载先起）
//! **这就是"铸入口那一枚必须长命"**）。故第 2 步那一问**等在门外**：门牌的号一开始还没有，
//! :tile 按额度重试（`RETRY_MS` 一拍问一次），持树者那边把它铺好了就答
//! # 判据为什么必须 **panic**
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

const MS: usize = 1000;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-control: ask open, three faces denied";

/// **一个一定不在装配表里的名字**：第 4 步那一问的荷载
/// 取"表里没有"是**故意**的：那一问要的就是"对面答得出一句语义码"。若拿一个真名字去问
/// 答案会跟着那一台的生命阶段变（同一句话量出两件事）
const NOBODY: &str = "probe-control-nobody";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**先装路**（次序是硬的，见文件头）。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-control: no tree link");
    };
    let tree = operator::Face::from(&session);

    // 二、树上那**一格**：名字（`seek`）→ 号 → **门牌那一枚**（`find` 会把它授进本表）。
    // 那段前缀（一块 `Pane`，没有门牌可授）。
    let road = ccall::DIR
        .try_join(ccall::Grant::State.name())
        .expect("probe-control: bad name");
    let plate = tree
        .tile(&road, Wait::AtMost(MS))
        .and_then(|tile| tile.token(Wait::AtMost(MS)));
    let Ok(entry) = plate else {
        panic!("probe-control: /svc/sys/control/state is not on the tree");
    };

    let Ok(control) = ccall::Face::of(entry) else {
        panic!("probe-control: plate refused as a control face");
    };

    // 四、**表里没有的名字** ⇒ 期望那一格语义码（`Unknown`）。这一条量的正是"这一问走到了对面、
    let nobody = NOBODY.to_string();
    let missing = control.service(nobody.clone()).state(Wait::AtMost(MS));
    debug!("probe-control: missing={missing:?}");

    // 五、**本台自己**：装配表里有这一行、而本台此刻活着 ⇒ 答得出一个生命阶段。
    let mine = "probe-control".to_string();
    let alive = control.service(mine).state(Wait::AtMost(MS));
    debug!("probe-control: self={alive:?}");

    // 六、**带规矩那三面**：`mint` / `start` / `stop` 各取一遍 ⇒ 期望 `Denied`（树那一层）。
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
