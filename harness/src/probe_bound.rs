#![no_std]
#![no_main]

//! probe-bound — **上界的证客**：一页 + 1 推不进去；不合族的帧不把门卡死。
//!
//! A 那一刀（消息孔一页封顶）落地时**没有真机读数**——本仓没有一位越界的推者（最大的帧是
//! `operator::ASK_MAX = 258`），故"超页被拒"当时只有契约与实现两个读者。本程序把那条判据搬到
//! 真机上：**一位故意的坏客人**。
//!
//! ```text
//!   1  与树开会话（`operator::open` + `ask_hole`）
//!   2  自铸一枚孔，推 **一页 + 1** 字节            ⇒ 期望 `Denied`
//!   3  那一枚孔照旧空着（`peek` 答 `Busy`）；再推一条 8 字节的 ⇒ 期望成（拒的是**长度**，
//!      不是这一枚孔坏了），`peek` 答 8
//!   4  往树的门上推 **300 字节的不合族帧** ⇒ 门把它取出来、答一句 `BAD`；随后一句正经的问
//!      （`part /sys`，幂等）照样答得出 —— **门没卡死**（这一格量的是一页缓冲那一刀）
//! ```
//!
//! # 为什么"坏客人"必须是一位真域
//!
//! 判据在核里（`Push` 的长度前置条件），而它的**执行**要一次真 envcall：宿主台上没有内核，
//! 喂假帧只能验用户侧的读法。故这一格只能由一台真域来撞——与 `probe-denied` 同一条路。
//!
//! # 第四条为什么先把它那声 `BAD` 读掉
//!
//! 树的门对**每一条取出来的帧都答一句**（读不懂答 `BAD`）——推了 junk 之后，那声 `BAD` 就落在
//! 本端的树路上。故这一条先把它读掉，再问正经的：留着不清，下一句问会读到上一句的答。
//!
//! **代价照实记**：真要是那道门卡死了，这一台会**堵在门外**（`push` 满则挂，核给的形状就是
//! "等"）——故红了会以"被期限砍断"的样子出现（`gate::stopped` 那一刀使它说得清），
//! 而不是以某一条断言的失败出现。
//!
//! # 照实记（这一台当场抓到的一处）
//!
//! 它一落地就红了：`operator` 那道门**漏在 A 那一刀之外**（前五处是 principal / coalition /
//! router / rtc / 板）——那道门的缓冲还是家族帧那么大（`ASK_MAX` = 258），于是 300 字节那一枚
//! 它取不出、也丢不掉，本端随后那句正经的问**堵在门外**。修法与前五处同一句（一页缓冲，起手
//! 备一次），见 `programs/src/supervisor/system/operator/server.rs` 的 `serve`。

extern crate alloc;
extern crate programs;

use programs::Report;

use alloc::format;
use alloc::vec::Vec;

use env::{Mark, Name, PieToken};
use harness::cases;
use protocol::system::operator::call as ocall;
use protocol::system::operator::client as operator;
use protocol::system::operator::{LINK, Where};
use protocol::session::Quay;
use runtime::PAGE_SIZE;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 等树 / 办一趟的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 本地失败编号（读数用）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
const OK_NOTE: &str = "probe-bound: bound held";

/// **一页 + 1**：刚好越界。再大也是同一个码（界是**一个区间**），但最小反例最读得清。
const OVER: usize = PAGE_SIZE + 1;

/// 不合族的帧：**在一页之内**，又不是这一族任何一条的形状（树那一边解不出来）。
const JUNK: usize = 300;

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(sire) = utask::sire() else { return bail("probe-bound: no sire") };

    // 一、与树开会话：本端那一枚交给生我者（它再转授给持树者），另铸一枚问话孔给它。
    let Ok((tree, host)) = operator::open(sire, MS) else { return bail("probe-bound: no tree link") };
    let Ok(hedge) = operator::ask_hole(host) else { return bail("probe-bound: no tree ask") };
    let Ok(dir) = Name::new("sys") else { return bail("probe-bound: bad name") };

    // 二、自铸一枚孔（**本域自己那一枚，没有读者**）——界那一格就在这里量。
    let Ok(hole) = mail::unseal_hole(Mark::of("probe-bound")) else { return bail("probe-bound: no hole") };
    let mine = mail::HolePie::from_token(hole);

    // 二·一、一页 + 1 ⇒ 期望拒。**那一页自己在堆上备**（一页这一档不住栈，与门那一侧同一句）。
    let mut big: Vec<u8> = Vec::new();
    if big.try_reserve_exact(OVER).is_err() {
        return bail("probe-bound: no room");
    }
    big.resize(OVER, 7);
    let over_code = match mine.push(&big) {
        Ok(()) => 0,
        Err(e) => e.source.code(),
    };
    drop(big);

    // 二·二、那一枚孔照旧空着；再推一条小的 ⇒ 该成。
    let empty = matches!(mine.peek(), Err(ref e) if e.source.is_busy());
    let small = mine.push(&[0u8; 8]).is_ok();
    let len = mine.peek().map(|(n, _)| n).unwrap_or(0);
    say(&format!(
        "probe-bound: push={over_code} empty={empty} small={small} len={len}"
    ));

    // 三、往树的门上推一枚不合族的帧，再看那道门还是不是活的。
    let (junk_in, said_bad, after) = junk_trip(hedge, &tree, dir);

    // 四、判据：**一例一条**，名字即结论。
    let mut suite = cases::Suite::new("probe-bound");
    suite.case("the_over_long_push_is_denied", move || {
        assert_eq!(over_code, -1, "一页 + 1 本该被拒（`Denied` = -1）");
    });
    suite.case("the_slot_keeps_nothing_from_the_refusal", move || {
        assert!(empty, "拒是拒了，可那一枚孔的槽里已经有东西了");
        assert!(small, "拒完之后再推一条 8 字节的也推不进去（这一枚孔坏了？）");
        assert_eq!(len, 8, "槽里那条不是刚推的那一条（长度 {len}）");
    });
    suite.case("a_foreign_frame_does_not_wedge_the_door", move || {
        assert!(junk_in, "不合族的帧推不进门（门那一枚孔不在？）");
        assert!(said_bad, "门没把那一条取出来 / 没答 `BAD`");
        assert!(after, "吞了 junk 之后，门不再答正经的问了");
    });
    suite.run();

    return Report::note(E_OK, OK_NOTE);
}

/// 第三条那一趟：**推 junk → 读掉它那声 `BAD` → 再问一句正经的**。
///
/// 返 `(推成了没有, 读到 BAD 没有, 正经的那一问答得出来没有)`——三格各是一件事，由调用方凑成
/// 一条判据（"这道门没卡死"）。
///
/// 正经那一问取 `part(/sys)`：**幂等**（`/sys` 是服务起手时立的那一格，重复 `part` 只答同一个
/// 号），故"答得出"就是这一条要的全部——答案对不对由别的证客管。
fn junk_trip(hedge: PieToken, tree: &Quay, dir: Name) -> (bool, bool, bool) {
    let junk = [0u8; JUNK];
    let pushed = mail::HolePie::from_token(hedge).push(&junk).is_ok();

    // 树路那一枚（本端的读口）：`ask_out` 那份答话就是从它读的。junk 那一声 `BAD` 先读掉。
    let mut back = [0u8; 8];
    let said = Name::new(LINK)
        .ok()
        .and_then(|at| tree.find(at))
        .and_then(|pier| pier.pull(&mut back, MS).ok());
    let bad = matches!(said, Some(1) if back[0] == ocall::BAD);

    // 正经的一问：**门还在答**。
    let after = operator::part(hedge, tree, Where::Root, dir, MS).is_ok();
    (pushed, bad, after)
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    say(note);
    return Report::note(E_TRIP, note);
}

/// 打一行。调试面是本域唯一的嘴（与 `echo` / `guest` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
