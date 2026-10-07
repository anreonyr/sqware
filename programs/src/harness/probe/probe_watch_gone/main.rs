#![no_std]
#![no_main]

//! probe-watch-gone — **量"退场即撤订"的那一半**：订一条路，停一下，**退场**。
//!
//! 判据不在本台（它自己什么都不判）：它造成的是"**一位订户没了**"这件事——
//! 此后同一条路上的改动，持树者往它那一枚孔上递手时会答 `Dead`／`Gone` ⇒ 那一行
//! `operator: watch dropped who=…` 与 `watchers=` 的下降就是判据（**读串口，不是整机那一格**）。
//!
//! 与 `probe-watch-after` **并发**：`after` 只等得到"对面答得动"，等不到"退场"——故靠对面
//! **连改几趟**把本台退场那一刻跨过去。改动**刻意少而稀**：那片负载会挤在驱动装配窗口里，
//! 把 `kernel/src/layout.rs` 在案的那族残余放大成红。

extern crate programs;

use core::time::Duration;

use env::Wait;
use programs::Report;

use system_api::operator::path::PathBuf;
use ipc::session::Session;
use system_client::operator::{Grant, Face as Face};
use system_client::operator::client as operator;
use env::unit;

const MS: usize = 1000;
/// 订的那条路 —— 与 `probe-watch-after` 改的那条**逐字相同**（两份文件各写一遍：各是独立 bin）。
const ROAD: &str = "svc/probe-swatch";
/// **订上之后还站多久**（毫秒）：够对面改头一两趟，然后本台走掉。
const HOLD_MS: u64 = 150;
const OK_NOTE: &str = "probe-watch-gone: subscribed then left";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(session) = Session::open(unit::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-watch-gone: no tree link");
    };
    let tree = Face::from(&session);
    let road = PathBuf::try_new(ROAD).unwrap_or_else(|| panic!("probe-watch-gone: bad road"));
    // **柄先绑**：临时的 `Rein` 活不过这一条绑定（与 `probe-watch` 那一手同形）。
    let rein = tree.rein(Grant::Watch);
    let watch = rein
        .watch(&road, Wait::AtMost(MS))
        .unwrap_or_else(|fail| panic!("probe-watch-gone: subscribe refused: {fail:?}"));
    let _ = watch.road();
    let _ = execution::room::park(Duration::from_millis(HOLD_MS));
    drop(watch);
    Report::note(env::EXIT_OK, OK_NOTE)
}
