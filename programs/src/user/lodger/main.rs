#![no_std]
#![no_main]

//! lodger — **房客**：占住一条线、**直接死**（不说再见）。
//!
//! 它是路由者那一手探活（`sweep`）的**读数程序**：`passer` 喂的是板那本账（"那一枚入口还答得
//! 出吗"），本域喂的是**线那本账**——它像一位驱动那样占住一条线，然后一句话不说就走。
//!
//! ```text
//!   1  领配给：`rtc@101000` 那枚 ONLY 门闩（真持有那台设备；本域从不映视图、不碰寄存器）
//!   2  上树一条会话：FIND /device/router ⇒ 那扇门
//!   3  occupy：报设备名（**线 = 名字的函数**），收一格答码 —— 那条线归本域
//!   4  报一行读数 `lodger: occupy=<码>`
//!   5  **直接死**：不说退场、不交回 ⇒ 它铸的那枚孔随退出钩子封印 ⇒ 路由者被叫醒、探活
//!      答不出 ⇒ 拆线 + 空出格子（读数 `router: vacate line=11`）
//! ```
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
//! 路由者解树解出来，本域从不说它（客户手里没有"线"）。
//!
//! # 特权级由清单定
//!
//! 本域是 **U 态**（`kernel/build.rs::INITRD_BINS`）：铸孔、交出、上树找服务、领一枚门闩
//! 都不需要 S 态。

extern crate alloc;
extern crate programs;

// 需求单归**收方**：本域那张单子住 lib 里（装配者要照它开单），同一份源码编一次。
// 客侧装配也共用驱动那一族那段机器（会话 + 收配给 + 归位）——它领门闩走的是同一条路。
use programs::driver::assemble;
use programs::user::lodger::needs;

// 树：本域是**客侧**（按名找服务）。
use protocol::operator::call as ocall;
use protocol::operator::client as operator;

use alloc::format;

use env::{Name, PieToken};
use protocol::driver::line;
use protocol::driver::line::call as lcall;
use protocol::session::Quay;
use runtime::env::debug;
use runtime::env::room::exit_with_note;
use runtime::env::unit as utask;

/// 本域要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 等树 / 办一趟登记的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 两种退场：占上了 / 没占上（都**不是 panic**；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 领配给：门闩到手就是"持有"的全部（本域不映视图、不碰寄存器）。缺格即装配错。
    let mut slots = [None; needs::WANTS.len()];
    let got = match assemble::receive(&mut slots, needs::slot_of) {
        Ok(n) => n,
        Err(code) => exit_with_note(code, "lodger: assemble"),
    };
    let [Some(_rtc)] = slots else {
        exit_with_note(assemble::E_GRANT, "lodger: no grant")
    };
    say(&format!("lodger: got {got}"));

    // 2/3. 上树找到线路由者，把本域那条线占住（止步于报设备名——线号是它解出来的）。
    let (code, _held) = occupy();
    say(&format!("lodger: occupy={code}"));

    // 4. **直接死**：不说退场那一句、不交回。`_held` 那条线活到本域退场为止——它铸的那枚孔
    //    随退出钩子封印，路由者那一格因此醒来（`router: vacate line=11`）。
    let ok = code == lcall::OK;
    exit_with_note(
        if ok { E_OK } else { E_TRIP },
        if ok { "lodger: gone" } else { "lodger: failed" },
    )
}

/// 占住本域那条线：从树上找到 `/device/router`，报设备名，收一格答码。
///
/// 答码用 [`lcall::code_of`]——**与线上同一张表**（客户端不从失败域另编一套号）。
/// 返的第二件是那条线本身：它活到本域退场（见 `main` 第 4 步）。
fn occupy() -> (u8, Option<line::client::Line>) {
    let Ok(sire) = utask::sire() else {
        return (lcall::BAD, None);
    };
    // 会话：同一个域只开一条（第二次 `open` 会撞同名，见 `driver/uart` 头注）。
    let Ok((link, host)) = operator::open(sire, MS) else {
        return (lcall::BAD, None);
    };
    let Ok(talk) = operator::ask_hole(host) else {
        return (lcall::BAD, None);
    };
    let (Ok(dir), Ok(want)) = (Name::new(protocol::driver::DIR), Name::new(SERVICE)) else {
        return (lcall::BAD, None);
    };
    let path = [dir, want];
    let none = PieToken::NONE;
    let code = operator::ask(talk, &link, host, ocall::FIND, &path, none, MS).unwrap_or(lcall::BAD);
    if code != ocall::OK {
        return (lcall::BAD, None);
    }
    let Some(entry) = operator::take(&link, host) else {
        return (lcall::BAD, None);
    };
    let Some(device) = needs::WANTS[0].name() else {
        return (lcall::BAD, None);
    };
    match line::client::Line::occupy(entry, device, MS) {
        Ok(held) => (lcall::OK, Some(held)),
        Err(fail) => (lcall::code_of(fail), None),
    }
}

/// 打一行。调试面是本域唯一的嘴（与 `guest` / `passer` 用的是同一格）。
fn say(msg: &str) {
    let _ = debug::put(msg);
}
