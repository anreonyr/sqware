#![no_std]
#![no_main]

//! probe-rule-other — **另一位客人**：**有身份**地去用别人立了规矩的那几格，期望被拒。
//!
//! `probe-rule` 那一台证的是"**规矩随身份走**"（同一个 TID 换一位代表，答案就变了）。
//! 而 `Rule::Is` 与 `Rule::Under` 各还有一格**只有另一台客人量得到**：
//!
//! - `Under(p)` 的负证要一位**不在 p 那一支里**的——同一台客人做不到：`q = derive(p)` 一定
//!   在 p 那一支里，而 `adopt` 只许**往下**领（`heir(current, q)`，见
//!   `protocol::system::principal::core` 的 `Principal::adopt` 三格前置）；
//! - `Is(p)` 的负证要一位**不是 p** 的——`probe-rule` 用 adopt 演过一次，本台再换**一台客人**
//!   演一次：装配期每位都是 `derive(ROOT)` 的**兄弟**，故彼此都不在对方那一支里。
//!
//! ```text
//!   1  上树、开一条问话孔（本域不需要门牌：只 `seek` + `find`，不问身份服务）
//!   2  SEEK /sys/rule/is      ⇒ FIND ⇒ 期望 DENIED(8)
//!   3  SEEK /sys/rule/under   ⇒ FIND ⇒ 期望 DENIED(8)
//!   4  SEEK /sys/rule/foreign ⇒ FIND ⇒ 期望 DENIED(8)
//!      —— 那一格许给的是"**开着 `/sys/principal` 那一格**的那位"（规矩由 `probe-rule` 落，
//!         按 `seek` 换来的号写），本域不是那一位 ⇒ 同样拒。**这一格不依赖次序**：那枚门牌
//!         的主人是常驻服务，整轮都活着。
//!   5  报一行读数就退场
//! ```
//!
//! # 为什么这两格也是必要的（`8` 与 `9` 与 `0` 三里必须落在 `8`）
//!
//! 门禁的答案有**三格**：`OK`（放行）/ `DENIED`（终态：换人、换目标，别重试）/
//! `UNJUDGED`（判不了——因分"会好的"与"好不了的"两类）。本台量的是中间那一格——
//! **有身份、但这一格不给你**。它若答成 `9`，整机就分不出"这一格不给你"；答成 `0`，门禁
//! 等于不存在。上一刀 `probe-denied` 量的是**第一道门**（有没有身份），本台量的是**第二道门**。
//!
//! 照实记：本台只 `seek` + `find`，**不 `land`**——落牌是"改这一格"那一轴（由 `probe-owner`
//! 那一台管），而那几格是 `mine = false` 落下的（谁都能改）。若本台顺手落一次，就会**顶掉**
//! `probe-rule` 那几格（改这一轴不归它管，可"落牌"本身会换绑），后面的读数就全变了。
//! `foreign` 那一格也是 `probe-rule` 落的——本台只负责"换一台客人再去撞一次"。

// 本文件是一份**独立的 bin**（`harness/Cargo.toml` 的 `prog-probe-rule-other`），**不进 lib**
// ——与 `echo` / `probe-denied` 同一条：`programs/src/user/mod.rs` 里没有它。
//
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use alloc::format;
use core::time::Duration;

use env::{Name, PieToken};
use protocol::session::Quay;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;
use runtime::env::debug;
use runtime::env::room;
use runtime::env::unit as utask;

const DIR: &str = "sys";
const PANE: &str = "rule";
const IS: &str = "is";
const UNDER: &str = "under";
/// `probe-rule` 落的第三格：规矩 = `Opens(/sys/principal 那一格)`（许给**别人**）。
const FOREIGN: &str = "foreign";

/// 等树 / 等答的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

/// 那一格可能落得比本域晚：找不到就再问一次的间隔（毫秒）。
const RETRY_MS: usize = 1;

/// 退场码：走通了 / 没走通（都不是 panic；kernel 会把那一行连同域号打出来）。
const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）。
///
/// **照实记（搬进用例之后）**：`BAD_NOTE`、以及"没走通"那条退场路，一起退役了——判据现在是
/// **一例一条**（`cases::Suite`），失败走 panic 通道、域当场死，故失败再也走不到出口那一手。
const OK_NOTE: &str = "probe-rule-other: all three denied as expected";

#[programs::entry]
fn main() -> Report<'static> {
    let Ok(sire) = utask::sire() else {
        return bail("probe-other: no sire");
    };

    // 一、上树：本域只开一条会话（不找门牌——本台只 `seek` / `find`，不问身份）。
    let Ok((tree, host)) = operator::open(sire, Wait::AtMost(MS)) else {
        return bail("probe-other: no tree link");
    };
    let Ok(talk) = operator::ask_hole(host) else {
        return bail("probe-other: no tree ask");
    };
    let (Ok(dir), Ok(pane), Ok(is_name), Ok(under_name), Ok(foreign_name)) = (
        Name::new(DIR),
        Name::new(PANE),
        Name::new(IS),
        Name::new(UNDER),
        Name::new(FOREIGN),
    ) else {
        return bail("probe-other: bad name");
    };

    // 二、按名字取号（**这一手不过门禁**：`seek` 不在闸口里），再 `find`——那几手该被拒。
    let is = denied(talk, &tree, &[dir, pane, is_name]);
    let under = denied(talk, &tree, &[dir, pane, under_name]);
    let foreign = denied(talk, &tree, &[dir, pane, foreign_name]);

    // 三、一行读数。
    say(&format!(
        "probe-other: tree is={is} under={under} foreign={foreign}"
    ));

    // 四、判据：**一例一条**——三格都恰是 `DENIED`（不是 `0` 放行，也不是 `9` 判不了）。
    {
        assert_eq!(is, ocall::DENIED)
    }
    {
        assert_eq!(under, ocall::DENIED)
    }
    {
        assert_eq!(foreign, ocall::DENIED)
    }

    return Report::note(E_OK, OK_NOTE);
}

/// 沿一条路译成号再 `find`：答线上那一格码（译不出号 ⇒ `UNKNOWN`）。
///
/// **带一轮有界重试**：`/sys/rule` 那几格由另一台客人落下，它可能落得比本域晚。
fn denied(talk: PieToken, link: &Quay, road: &[Name]) -> u8 {
    let mut left = MS;
    let id = loop {
        match operator::seek(talk, link, road, Wait::AtMost(MS)) {
            Ok(id) => break id,
            Err(ocall::UNKNOWN) if left > 0 => {
                let _ = room::sleep(Duration::from_millis(RETRY_MS as u64));
                left = left.saturating_sub(RETRY_MS);
            }
            Err(code) => return code,
        }
    };
    // `find` 的失败域是 `Fail`（六格），答话码是另一张表——这里只关心"拒没拒"，
    // 故译不出号/推不动都按 [`ocall::BAD`] 记（读数上分得开）。
    // （`find` 的第二格 = 那一枚入口在本域表里的号：这一支不看它，只要状态。）
    operator::find(talk, link, id, Wait::AtMost(MS))
        .map(|(code, _entry)| code)
        .unwrap_or(ocall::BAD)
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
