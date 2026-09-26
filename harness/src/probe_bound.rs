#![no_std]
#![no_main]

//! probe-bound — **上界的证客**：一页 + 1 推不进去；不合族的帧不把门卡死。
//!
//! A 那一刀（消息孔一页封顶）落地时**没有真机读数**——本仓没有一位越界的推者（最大的帧是
//! `operator::REQ_LEN = 258`），故"超页被拒"当时只有契约与实现两个读者。本程序把那条判据搬到
//! 真机上：**一位故意的坏客人**。
//!
//! ```text
//!   1  与**两道门**开会话（`board::open` ＋ `operator::open`，各自一枚 `ask_hole`）——两条路
//!      都要赶在装配那一步的期限之内装上（次序＝装配者那一侧：板在前，树在后）
//!   2  自铸一枚孔，推 **一页 + 1** 字节            ⇒ 期望 `Denied`
//!   3  那一枚孔照旧空着（`peek` 答 `Busy`）；再推一条 8 字节的 ⇒ 期望成（拒的是**长度**，
//!      不是这一枚孔坏了），`peek` 答 8
//!   4  往树的门上推 **300 字节的不合族帧** ⇒ 门把它取出来、答一句 `BAD`；随后一句正经的问
//!      （`part /sys`，幂等）照样答得出 —— **门没卡死**（这一格量的是一页缓冲那一刀）
//!   5  **板那一道门**同一句：推 300 字节 ⇒ 答 `BAD`；随后 `evict`（一字节短帧，空载荷）
//!      照样答得出 —— 两道门各有一条腿（第二处的来历见下）
//! ```
//!
//! # 为什么"坏客人"必须是一位真域
//!
//! 判据在核里（`Push` 的长度前置条件），而它的**执行**要一次真 envcall：宿主上没有内核，
//! 喂假帧只能验用户侧的读法。故这一格只能由一台真域来撞——与 `probe-denied` 同一条路。
//!
//! # 第四条为什么先把它那声 `BAD` 读掉
//!
//! 树的门对**每一条取出来的帧都答一句**（读不懂答 `BAD`）——推了 junk 之后，那声 `BAD` 就落在
//! 本端的树路上。故这一条先把它读掉，再问正经的：留着不清，下一句问会读到上一句的答。
//! **板那一道门同款**（第五条的 `evict` 也先读掉 junk 那一声）。
//!
//! **代价照实记**：真要是那道门卡死了，这一台会**堵在门外**（`push` 满则挂，核给的形状就是
//! "等"）——故红了会以"被期限砍断"的样子出现（`gate::stopped` 那一刀使它说得清），
//! 而不是以某一条断言的失败出现。
//!
//! # 照实记（这一台当场抓到的那一处）
//!
//! 它一落地就红了：`operator` 那道门**漏在 A 那一刀之外**（前五处是 principal / coalition /
//! router / rtc / 板）——那道门的缓冲还是家族帧那么大（`operator::REQ_LEN` = 258），于是 300 字节那一枚
//! 它取不出、也丢不掉，本端随后那句正经的问**堵在门外**。修法与前五处同一句（一页缓冲，起手
//! 备一次），见 `programs/src/system/operator/server.rs` 的 `serve`。
//!
//! # 照实记（第五条那一腿：板那一门在 ④ 那一程里被退掉的一页）
//!
//! 上面那句"前五处……板"是 **A 那一刀**的账：板那道门当时**有**一页。而 ④ 那一程的刀一
//! （`Message` ＋ `Slip` 立起来）把它的收帧换成 `Slip::land`，缓冲给的是**家族帧那么大**的一
//! 只，**顺手把那一页退掉了**，并在源码里写下"那一页不再需要"——**那句话是假的**：拿家族帧那
//! 么大的一只收不下更长的那一条，而核**不丢**取不出的消息 ⇒ 那一枚永远留在槽里，组每轮唤醒、
//! 门每轮答一句 `BAD`，客人下一次正经的推堵在门外。第五条就是那一句的判据（修法：给 `land`
//! 传**载体那一页** ＋ 把那一页请回来）。

extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::format;
use alloc::vec::Vec;

use env::{Mark, Name, PieToken};
use protocol::session::Quay;
use protocol::system::board as bcall;
use protocol::system::board::client as board;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use protocol::system::operator::{LINK, Where};
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

/// 不合族的帧有多长：**在一页之内**，又不是这一族任何一条的形状。
const JUNK: usize = 300;

/// 那一条的首格：**第一条动作码**（树那一族的 `LAND` ＝ 板那一族的 `REGISTER` ＝ `1`），
/// 其余全零——于是两道门都走到"动作认得、形状不对"那一支 ⇒ 各答一句 `BAD`。
///
/// **照实记（第一版拿全零当 junk，读数当场是错的）**：全零那一条在**板**那一族解出来的是
/// `Wire::Unknown`（`0` 是"表外的动作码"——读得懂，只是那一码不是那四枚之一），故板答的是
/// `UNKNOWN` 而不是 `BAD`，这一台红在"板没答 `BAD`"上。**是这一条判据写错了，不是门坏了**：
/// 两道门的读法本来就不同（板那一族多一格"表外的码"，树那一族没有）。
const JUNK_OP: u8 = 1;

/// 编那一条 junk（点数在 [`JUNK`]，首格在 [`JUNK_OP`]）。
fn junk() -> [u8; JUNK] {
    let mut junk = [0u8; JUNK];
    junk[0] = JUNK_OP;
    junk
}

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(sire) = utask::sire() else {
        return bail("probe-bound: no sire");
    };

    // 一、**两条路先都装上**：本端那一枚交给生我者（它再转授给对方），另铸一枚问话孔给它。
    //
    // **次序＝装配者那一侧的次序**（板在前、树在后），而两条都**必须赶在装配那一步的期限
    // 之内**：装配者按行装完就把这一位的路 `claim` 下来（有期限），故这一台**不能先做别的
    // 手脚再装路**——照实记：第一版把树那一条腿（含 junk 那一趟的两个有界等）排在装路之前，
    // 装配那一侧当场报 `board:claim`（`步骤` 读数），这一台连树路都没拿到。
    let Ok((deck, seat)) = board::open(sire, Wait::AtMost(MS)) else {
        return bail("probe-bound: no board link");
    };
    let Ok(bolt) = board::ask_hole(seat) else {
        return bail("probe-bound: no board ask");
    };
    let Ok((tree, host)) = operator::open(sire, Wait::AtMost(MS)) else {
        return bail("probe-bound: no tree link");
    };
    let Ok(hedge) = operator::ask_hole(host) else {
        return bail("probe-bound: no tree ask");
    };
    let Ok(dir) = Name::new("sys") else {
        return bail("probe-bound: bad name");
    };

    // 二、自铸一枚孔（**本域自己那一枚，没有读者**）——界那一格就在这里量。
    let Ok(hole) = mail::unseal_hole(Mark::of("probe-bound")) else {
        return bail("probe-bound: no hole");
    };
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

    // 三·五、**板那一道门**：同一条判据的另一条腿（来历见文件头那一段照实记）。
    let (b_junk_in, b_said_bad, b_after) = junk_trip_board(bolt, &deck);

    // 四、判据：**一例一条**，名字即结论。
    {
        {
            assert_eq!(over_code, -1, "一页 + 1 本该被拒（`Denied` = -1）");
        }
    }
    {
        {
            assert!(empty, "拒是拒了，可那一枚孔的槽里已经有东西了");
            assert!(
                small,
                "拒完之后再推一条 8 字节的也推不进去（这一枚孔坏了？）"
            );
            assert_eq!(len, 8, "槽里那条不是刚推的那一条（长度 {len}）");
        }
    }
    {
        {
            assert!(junk_in, "不合族的帧推不进门（门那一枚孔不在？）");
            assert!(said_bad, "门没把那一条取出来 / 没答 `BAD`");
            assert!(after, "吞了 junk 之后，门不再答正经的问了");
        }
    }
    {
        {
            assert!(b_junk_in, "不合族的帧推不进板那道门（那一枚孔不在？）");
            assert!(b_said_bad, "板没把那一条取出来 / 没答 `BAD`");
            assert!(b_after, "吞了 junk 之后，板不再答正经的问了");
        }
    }

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
    let junk = junk();
    let pushed = mail::HolePie::from_token(hedge).push(&junk).is_ok();

    // 树路那一枚（本端的读口）：`ask_out` 那份答话就是从它读的。junk 那一声 `BAD` 先读掉。
    let mut back = [0u8; 8];
    let said = Name::new(LINK)
        .ok()
        .and_then(|at| tree.find(at))
        .and_then(|pier| pier.pull(&mut back, Wait::AtMost(MS)).ok());
    let bad = matches!(said, Some(1) if back[0] == ocall::BAD);

    // 正经的一问：**门还在答**。
    let after = operator::part(hedge, tree, Where::Root, dir, Wait::AtMost(MS)).is_ok();
    (pushed, bad, after)
}

/// 第五条那一趟（**板那一道门**）：与 [`junk_trip`] 逐字同一句判据，换一道门、换一句正经的问。
///
/// 正经那一问取 `evict`（**一字节短帧、空载荷**）：这一位没在板上登记过 ⇒ 板答 `UNKNOWN`
/// ——"答得出"就是这一条要的全部（答得对不对由别的证客管），而它**不铸孔、不交入口**，
/// 故这一条量的是**门**，不是账。
fn junk_trip_board(bolt: PieToken, deck: &Quay) -> (bool, bool, bool) {
    let junk = junk();
    let pushed = mail::HolePie::from_token(bolt).push(&junk).is_ok();

    // 板那一路那一枚（本端的读口）：junk 那一声 `BAD` 先读掉。
    let mut back = [0u8; 8];
    let said = Name::new(bcall::LINK)
        .ok()
        .and_then(|at| deck.find(at))
        .and_then(|pier| pier.pull(&mut back, Wait::AtMost(MS)).ok());
    let bad = matches!(said, Some(1) if back[0] == bcall::BAD);

    // 正经的一问：**门还在答**。
    let after = board::evict(bolt, deck, Wait::AtMost(MS)).is_ok();
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
