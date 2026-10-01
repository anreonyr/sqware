#![no_std]
#![no_main]

//! probe-watch-after — **量"退场即撤订"的另一半**：在 `probe-watch-gone` 订过的那条路上**连改几趟**。
//!
//! 判据不在本台（它只保证那几趟改得成、树还答得动）：判据是持树者那一侧的两行读数——
//! **`operator: watch dropped who=…`**（那一位没了 ⇒ 就地摘）与 `watchers=` 的下降。
//!
//! **为什么连改而不是改一趟**：`after` 只能等到"对面答得动"，等不到"对面退场"——故两台并发，
//! 靠**跨过对面退场那一刻**来保证至少有一趟落在"那位订户已经没了"之后。
//! **改的是同一格**（`Rebound`）：不占窗格容量（一格窗格最多 `PANE_CAP` 个子——第一版在这里
//! 栽过：连落 40 个新名字，第 16 趟答 `Full`，读数被一句"tree gone?"带偏）。
//! **趟数少、间隔长**：那片负载会挤在驱动装配窗口里，把 `kernel/src/layout.rs` 在案的
//! 那族残余（树单线程、客人有界等待被排在别人后头）放大成红。

extern crate alloc;
extern crate programs;

use alloc::string::ToString;
use core::time::Duration;

use env::Wait;
use programs::Report;

use protocol::common::path::PathBuf;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::{Face as TreeFace, Mine};
use protocol::service::operator::Permit;
use runtime::env::mail;
use runtime::env::room;
use runtime::env::unit as utask;

const MS: usize = 1000;
/// 那一块窗格 —— 与 `probe-watch-gone` 订的那条路**逐字相同**（两份文件各写一遍）。
const PARENT: &str = "probe-swatch";
/// 窗格底下**那一格的名字**：本台连改的就是它（不新增子格 ⇒ 不碰 `PANE_CAP`）。
const IN: &str = "in";
/// 连改几趟（每一趟都匹配那位订户的过滤）。
const CELLS: usize = 6;
/// 两趟之间隔多久（毫秒）：跨过对面退场那一刻，同时**别挤**。
const STEP_MS: u64 = 200;
const OK_NOTE: &str = "probe-watch-after: rebound six times";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(session) = Session::open(utask::sire(), operator::BERTH, Wait::AtMost(MS)) else {
        panic!("probe-watch-after: no tree link");
    };
    let tree = TreeFace::from(&session);
    let road = PathBuf::try_new(PARENT).unwrap_or_else(|| panic!("probe-watch-after: bad road"));

    // 一、立那块窗格（**这一手也匹配那位订户的过滤**：过滤是前缀）。
    let root = tree.root();
    let Ok(pane) = root.open(PARENT.to_string(), Wait::AtMost(MS)) else {
        panic!("probe-watch-after: open {PARENT} failed");
    };

    // 二、连改同一格：每一趟都是一次真改动（`Rebound`）。
    for i in 0..CELLS {
        let Ok(entry) = mail::unseal_hole(env::Mark::of("probe-swatch-after")) else {
            panic!("probe-watch-after: no entry");
        };
        match pane.bind(IN.to_string(), entry, Permit::Unset, Mine::No, Wait::AtMost(MS)) {
            Ok(tile) => debug!("probe-watch-after: rebound #{i} id={}", tile.id().get()),
            Err(fail) => panic!("probe-watch-after: 第 {i} 趟改那一格失败：{fail:?}"),
        }
        // **树还答得动吗**：同一块窗格再问一次。
        let Ok(again) = tree.pane(&road, Wait::AtMost(MS)) else {
            panic!("probe-watch-after: the tree stopped answering after #{i}");
        };
        drop(again);
        let _ = room::sleep(Duration::from_millis(STEP_MS));
    }

    let Ok(listing) = pane.list(Wait::AtMost(MS)) else {
        panic!("probe-watch-after: the tree stopped answering (list)");
    };
    let seen = listing.iter().count();
    assert_eq!(seen, 1, "那块窗格底下该只有 1 格（本台连改同一格），数到 {seen}");
    Report::note(env::EXIT_OK, OK_NOTE)
}
