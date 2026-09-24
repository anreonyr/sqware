#![no_std]
#![no_main]

//! lodger — **房客**：占住一条线、**直接死**（不说再见）。
//!
//! 它是路由者那一手探活（`sweep`）的**读数程序**：`passer` 喂的是板那本账（"那一枚入口还答得
//! 出吗"），本域喂的是**线那本账**——它像一位驱动那样占住一条线，然后一句话不说就走。
//!
//! ```text
//!   1  领配给：**按类 `virtio,mmio` 要**（本机八台同类，编排域取首址最小的那台 =
//!      `virtio_mmio@10001000`，1 号线）那枚 ONLY 门闩——真持有那台设备，但本域
//!      从不映视图、不碰寄存器（为什么要一条**没人要**的线，见 [`needs`]）
//!   2  上树一条会话：FIND /device/router ⇒ 那扇门
//!   3  三趟登记 —— 成功那一格与**失败域**都卖读数（答码见 `line::call` 那张表）：
//!        占那条 virtio 线    → `lodger: occupy=0`    （0 = OK：线归本域）
//!        同一条线再来一次     → `lodger: taken=2`     （2 = TAKEN：主人是本域自己）
//!        报一件不是中断源的东西 → `lodger: unknown=1`   （1 = UNKNOWN：源表里没有那个坐标——门铃）
//!   4  **直接死**：不说退场、不交回 ⇒ 它铸的那枚孔随退出钩子封印 ⇒ 路由者被叫醒、探活
//!      答不出 ⇒ 拆线 + 空出格子（读数 `router: vacate line=1`）。死之前报一行 `lodger: pies=`
//!      ——**失败那两趟两边收干净了没有**的读数（见下面那一注）。
//! ```
//!
//! # `TAKEN` 那一趟为什么拿本域自己的线试
//!
//! 拿 `uart` 那条线试会**与它的登记抢时间**（装配单里 `uart` 排在本域之前，但它的登记在本域
//! 之后才办完）——谁先到谁得 `OK`，那是竞态，不是读数。拿**本域刚占下的那条线**再来一次，
//! 答 `TAKEN` 就是**确定**的，而判据一字不改（"这条线有人了"——主人是谁不影响这一格）。
//!
//! # 为什么它要真领那枚门闩
//!
//! 线路由者**不验属主**（那是照实记下来的代价，见 [`protocol::driver::line`]），故"只报名、
//! 不领设备"一样占得住线。本域**真领**：这样"主人没了"这句话才是字面意义上真的——它确实持有
//! 那台设备，只是从不碰它（需求单上因此只要最小的一格权）。
//!
//! # 名字与线号
//!
//! 设备名只有一处（[`needs`] 那张单子），与 `uart` 同一条纪律：**本域不发明名字**；线号由
//! 路由者解树解出来，本域从不说它（客户手里没有"线"）。唯一一个本域自己编的名字是那趟
//! `UNKNOWN` 的探针名——它**故意**不是任何节点（"解树答不出"说的就是这个）。
//!
//! # 特权级由清单定
//!
//! 本域是 **U 态**（`env::assembly::ALL` 里这一行的 `kind`）：铸孔、交出、上树找服务、领一枚门闩
//! 都不需要 S 态。

extern crate alloc;
extern crate programs;

use programs::Report;

// 需求单归**收方**：本域那张单子住 lib 里（装配者要照它开单），同一份源码编一次。
// 客侧装配也共用驱动那一族那段机器（会话 + 收配给 + 归位）——它领门闩走的是同一条路。
use programs::driver::assemble;
use harness::lodger::needs;

// 树：本域是**客侧**（按名找服务）。
use protocol::operator::call as ocall;
use protocol::operator::client as operator;

use alloc::format;

use env::{Key, Name, PieToken};
use protocol::driver::line;
use protocol::driver::line::call as lcall;
use cases::Suite;
use runtime::env::debug;
use runtime::env::mail;
use runtime::env::unit as utask;

/// 本域要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 两种退场：三趟都答对了 / 有一趟不是（都**不是 panic**；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[programs::entry]
fn main() -> Report<'static> {
    // 1. 领配给：门闩到手就是"持有"的全部（本域不映视图、不碰寄存器）。缺格即装配错。
    let mut slots = [None; needs::WANTS.len()];
    let got = match assemble::receive(&mut slots) {
        Ok(n) => n,
        Err(code) => return Report::note(code, "lodger: assemble"),
    };
    let [Some(grant)] = slots else {
        return Report::note(assemble::E_GRANT, "lodger: no grant");
    };
    say(&format!("lodger: got {got}"));

    // 2. 上树一条会话：找到线路由者，取回那扇门——三趟登记都往它推（会话一个域只开一条）。
    let entry = match find_router() {
        Some(entry) => entry,
        None => return Report::note(E_TRIP, "lodger: no router"),
    };
    // 坐标**随记录发下来**（本域不写死它；"这一类是哪一台"由编排域读树定）。
    let Some(key) = grant.key() else {
        return Report::note(E_TRIP, "lodger: no key");
    };

    // 3. 三趟登记：占上 / 同一条线再来一次 / 报一件**不是中断源**的东西。
    let (ok, held) = attempt(entry, key);
    say(&format!("lodger: occupy={ok}"));
    let (taken, _) = attempt(entry, key);
    say(&format!("lodger: taken={taken}"));
    // 第三趟报**门铃**那一形：坐标本身是合法的（内核真造过它），而**线只挂在设备上**
    // ——路由者在它那张源表里找不到这个坐标，答 `UNKNOWN`（1）。
    let unknown = attempt(entry, Key::irq()).0;
    say(&format!("lodger: unknown={unknown}"));

    // 4. **直接死**：不说退场那一句、不交回。`held` 那条线活到本域退场为止——它铸的那枚孔
    //    随退出钩子封印，路由者那一格因此醒来（`router: vacate line=1`）。
    let _held = held;
    // 三趟之后本域表里还剩几枚：**失败那两趟两边收干净了没有**的读数——`TAKEN` 与 `UNKNOWN`
    // 各把本端 `seat` 出去的那一枚（`Quay::shut`）与本趟借出去的那枚回信孔放下（见
    // `protocol::driver::line::client::Line::occupy`）。少放一枚，这一格当场大 1。
    let pies = mail::table_size();
    say(&format!("lodger: pies={pies}"));

    // 判据就地登记（用户裁定"服务台搬进 SUT"）：**只搬本域已经在判的东西**。前三例的期望是
    // 三趟登记的答码（与读数同一批常量）；第四例 `pies=9` 是**探针良过的那一格**（头注：把失败
    // 那两趟的释放临时关掉，同一处读数从 9 变成 14）——故它是一个**判据**，不是常数（少放一枚
    // 孔，这一例就红）。旧宿主靶上 `lodger: occupy=0` / `taken=2` / `unknown=1` / `pies=9` 钉的
    // 就是这四样。
    let mut suite = Suite::new("lodger");
    suite.case("the_line_is_mine", move || assert_eq!(ok, lcall::OK));
    suite.case("the_same_line_twice_is_taken", move || {
        assert_eq!(taken, lcall::TAKEN)
    });
    suite.case("a_bell_is_not_an_interrupt_source", move || {
        assert_eq!(unknown, lcall::UNKNOWN)
    });
    suite.case("the_failed_attempts_left_no_holes", move || {
        assert_eq!(pies, 9)
    });
    suite.run();

    let all = ok == lcall::OK && taken == lcall::TAKEN && unknown == lcall::UNKNOWN;
    return Report::note(
        if all { E_OK } else { E_TRIP },
        if all {
            "lodger: gone"
        } else {
            "lodger: failed"
        },
    )
}

/// 上树一趟：`FIND /device/router` ⇒ 那扇门（登记从它走）。
fn find_router() -> Option<PieToken> {
    let sire = utask::sire().ok()?;
    let (link, host) = operator::open(sire, MS).ok()?;
    let talk = operator::ask_hole(host).ok()?;
    let dir = Name::new(protocol::driver::DIR).ok()?;
    let want = Name::new(SERVICE).ok()?;
    let road = [dir, want];
    // **间接寻址那一手**：名字先译成号，此后按号。
    let id = operator::seek(talk, &link, &road, MS).ok()?;
    if operator::find(talk, &link, id, MS).unwrap_or(ocall::BAD) != ocall::OK {
        return None;
    }
    operator::take(&link, host)
}

/// 占一趟：报**那一段区**、收一格答码。返的第二件是那条线本身（占上了才有）。
///
/// 答码用 [`lcall::fail_to_code`]——**与线上同一张表**（客户端不从失败域另编一套号）。
fn attempt(entry: PieToken, key: Key) -> (u8, Option<line::client::Line>) {
    match line::client::Line::occupy(entry, key, MS) {
        Ok(held) => (lcall::OK, Some(held)),
        Err(fail) => (lcall::fail_to_code(Some(fail)), None),
    }
}

/// 打一行。调试面是本域唯一的嘴（与 `guest` / `passer` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}

