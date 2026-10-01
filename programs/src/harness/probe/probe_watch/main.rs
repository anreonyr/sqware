#![no_std]
#![no_main]

//! probe-watch — **事件那条路的正证客人**：持 `watch` 与 `land` 两位，订一条子树，再自己往树上
//! 落两格（一棵在订的范围里、一棵不在）。
//!
//! 判据四条，一例一条：
//! 1. **到得对**：订的范围里那一格落下之后，**等铃一次**就收到一条事件（不轮询、不重试）；
//! 2. **路与号对得上**：那条事件里的 `road` 正是那一棵、`id` 正是那一格自己的号；
//! 3. **过滤**：订的那条路**之外**那一格落下 ⇒ 收不到（紧接着一等期限内没有第二条）；
//! 4. **种类**：落的是一条 `Kind::Landed`（新铸），不是 `Rebound` / `Parted` / `Trimmed`。
//!
//! # 为什么这一台要自持两位（两条会话）
//! 一位客人一条会话是本族既有的形状（认领键是"谁开的 ＋ 树路记号"）——`watch` 与 `land` 是
//! **两柄权**，故两条会话。这样这一台不必借别人铺试验场：它落的每一步都由它自己报得出坐标。
//!
//! # 判据为什么必须 **panic**
//! 整机那一格判的是"有没有 `EXIT_PANIC`"（`kernel/src/work/room/conductor.rs` 的判据只此一处）：
//! 返回一个非零的 `Report` **不算红**。故每一步失败都当场塌。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;

use env::Wait;
use programs::Report;

use protocol::common::path::PathBuf;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine, Pane};
use protocol::service::operator::frame::watch::{Event, Kind};
use protocol::service::operator::{EntryId, Grant, Permit};
use runtime::env::mail;
use runtime::env::unit as utask;

const MS: usize = 1000;

/// **等事件**的额度（毫秒）：铃响是提示型，收到即醒；给足装配窗口但不做成轮询
const WAIT_MS: usize = 3_000;

/// 订的那条路（树底下一段，够短）
const IN_ROAD: &str = "probe-watch/in";
/// **不在订的范围里**的那一条（第 3 条判据用它）
const OUT_ROAD: &str = "probe-watch/out";

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-watch: landed=1 filtered=1";

#[programs::entry]
fn main() -> Report<'static> {
    // 一、**一条会话**（`Face::root()` 那条全操作面的路，与 `probe-rule` 同一手）：
    //    订与落都走它。**一位客人一条会话**是本族的形状（认领键是"谁开的 ＋ 树路记号"），
    //    故这一台不另开第二条——两条会话同开时，第二位客人认不到自己的那条答话路。
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-watch: no tree link");
    };
    let tree = TreeFace::from(&session);

    // 二、**订**：`watch` 那一面回 `OK` 之后，此后真变了才发得过来。
    let road = PathBuf::try_new(IN_ROAD).unwrap_or_else(|| panic!("probe-watch: bad road"));
    // **柄先绑**：`Rein` 是"借这一面借出来的那一柄权"，临时的 `Rein` 活不过这一条绑定。
    let rein = tree.rein(Grant::Watch);
    let mut watch = rein
        .watch(&road, Wait::AtMost(WAIT_MS))
        .unwrap_or_else(|fail| panic!("probe-watch: subscribe refused: {fail:?}"));

    debug!("probe-watch: subscribed road={}", watch.road());

    // 三、**订的范围里落一格** ⇒ 等一条事件（等铃，不轮询）。
    let inside = spot(&tree, IN_ROAD, "probe-watch-in");
    let got = wait_event(&mut watch, "in");
    assert_eq!(got.kind, Kind::Landed, "落下来那一格该报 Landed");
    assert_eq!(
        got.road.as_str(),
        IN_ROAD,
        "事件里那条路不对（收到的是 {}）",
        got.road
    );
    assert_eq!(got.id, inside, "事件里那一号不是刚落的那一格");
    debug!(
        "probe-watch: in id={} road={} kind={:?}",
        got.id.get(),
        got.road,
        got.kind
    );

    // 四、**订的范围外落一格** ⇒ 期限内收不到第二条（过滤）。
    let outside = spot(&tree, OUT_ROAD, "probe-watch-out");
    let quiet = watch.next(Wait::AtMost(300));
    assert!(
        quiet.is_err(),
        "订的那条路之外的改动也发过来了（id={}）：过滤没生效",
        outside.get()
    );
    debug!("probe-watch: out id={} silent=ok", outside.get());

    // 五、**那一页上就这一条**（没有多出来的）：再探一次仍是空。
    let none = watch.try_next();
    assert!(
        matches!(none, Ok(None)),
        "架上多出了没读过的那一条：{none:?}"
    );

    return Report::note(env::EXIT_OK, OK_NOTE);
}

/// **等一条事件**（等到就返；额度走完就当场红——"没到"与"到错了"都是红）。
fn wait_event(watch: &mut operator::Watch<'_>, what: &str) -> Event {
    if let Ok(Some(got)) = watch.try_next() {
        return got;
    }
    match watch.next(Wait::AtMost(WAIT_MS)) {
        Ok(got) => got,
        Err(fail) => panic!("probe-watch: {what} 那条事件没等到（{WAIT_MS} ms，{fail:?}）"),
    }
}

/// 在**根**底下按整条路落一格，答它自己的号。
///
/// 前缀那几段由**本台自己分**（`Pane::open`：幂等，缺的就地造）——那几段是这一台自己的试验场，
/// 不是别人的。**逐段只收号**（`Pane` 借的是上一块，攒着它就没法在循环里换新的）；
/// 落完再沿整条路译一遍，确认"事件里那个号"与树上那一格是同一个。
fn spot(tree: &TreeFace, road: &str, mark: &'static str) -> EntryId {
    let (parent, leaf) = match road.rsplit_once('/') {
        Some((parent, leaf)) => (parent, leaf.to_string()),
        None => ("", road.to_string()),
    };
    let pane = pane_at(tree, parent);
    let Ok(entry) = mail::unseal_hole(env::Mark::of(mark)) else {
        panic!("probe-watch: no entry for {road}");
    };
    let mut left = WAIT_MS;
    loop {
        match pane.bind(leaf.clone(), entry, Permit::Unset, Mine::No, Wait::AtMost(MS)) {
            Ok(id) => {
                let whole = PathBuf::try_new(road).unwrap_or_else(|| panic!("probe-watch: bad road"));
                match tree.tile(&whole, Wait::AtMost(MS)) {
                    Ok(tile) if tile.id() == id.id() => return id.id(),
                    Ok(tile) => panic!(
                        "probe-watch: {road} 译成 {} 而刚落的是 {}",
                        tile.id().get(),
                        id.id().get()
                    ),
                    Err(fail) => panic!("probe-watch: {road} 落完却译不出：{fail:?}"),
                }
            }
            Err(protocol::service::operator::Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(20));
                left = left.saturating_sub(20);
            }
            Err(fail) => panic!("probe-watch: bind {road} failed: {fail:?}"),
        }
    }
}

/// 把一条**容器路**走到那一块窗格上（逐段 `open`：幂等，缺的就地造）。
///
/// 空路 = 根。**换柄那一格靠"重开"**：`at` 是当前那一块，`.open(seg)` 答的是它的孩子
/// （借的是 `at`）——故先取号、再换柄。
fn pane_at<'t>(tree: &'t TreeFace, parent: &str) -> Pane<'t> {
    let mut at = tree.root();
    for seg in parent.split('/').filter(|one| !one.is_empty()) {
        let id = at
            .open(seg.to_string(), Wait::AtMost(MS))
            .unwrap_or_else(|fail| panic!("probe-watch: open {seg} failed: {fail:?}"))
            .id();
        at = Pane::of(tree, id);
    }
    at
}
