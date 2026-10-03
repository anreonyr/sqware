#![no_std]
#![no_main]

//! probe-rule-other — 另一位客人：有身份地去用别人立了规矩的那几格，期望被拒。
//! `probe-rule` 那一台证的是"**规矩随身份走**"（同一个 TID 换一位代表，答案就变了）。
//! 而 Exact 与 DescendantOf 各还有一格**只有另一台客人量得到**：
//!   在 p 那一支里，而 `adopt` 只许**往下**领（`heir(current, q)`，见
//! :core 的 Principal::adopt 三格前置）；
//!   演一次：装配期每位都是 `derive(ROOT)` 的**兄弟**，故彼此都不在对方那一支里。
//! # 为什么这两格也是必要的（`8` 与 `9` 与 `0` 三里必须落在 `8`）
//! 门禁的答案有**三格**：`OK`（放行）/ `DENIED`（终态：换人、换目标，别重试）/
//! `UNJUDGED`（判不了——因分"会好的"与"好不了的"两类）。本台量的是中间那一格——
//! 等于不存在。上一刀 `probe-denied` 量的是**第一道门**（有没有身份），本台量的是**第二道门**。
//! 那一台管），而那几格是 `mine = false` 落下的（谁都能改）。若本台顺手落一次，就会**顶掉**
//! `foreign` 那一格也是 `probe-rule` 落的——本台只负责"换一台客人再去撞一次"。

// ——与 `canonical` / `probe-denied` 同一条：`programs/src/user/mod.rs` 里没有它。
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use protocol::common::path::Path;
use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::Fail;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Face as TreeFace;
use runtime::env::unit as utask;

/// **容器那一段那一条路**（`/svc`）——那一段名字只在协议那一侧说（见 `probe_lease` 同款）
const DIR: &protocol::system::operator::Path = protocol::common::svc::SVC;
const PANE: &str = "rule";
const IS: &str = "is";
const UNDER: &str = "under";
/// `probe-rule` 落的第三格：规矩 = `Opener(/svc/sys/identity/resolve 那一格)`（许给**别人**）
const FOREIGN: &str = "foreign";

const MS: usize = 1000;

const E_OK: usize = 0;
const E_TRIP: usize = 1;

/// 走通那一句（不是 panic；kernel 会把这一句连同域号打出来）
const OK_NOTE: &str = "probe-rule-other: all three denied as expected";

#[programs::entry]
fn main() -> Report<'static> {
    let sire = utask::sire();

    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-other: no tree link");
    };
    let tree = TreeFace::of(session);
    let Some(base) = DIR.try_join(PANE) else {
        return bail("probe-other: bad name");
    };

    let is = denied(&tree, &base, IS);
    let under = denied(&tree, &base, UNDER);
    let foreign = denied(&tree, &base, FOREIGN);

    // 三、一行读数。
    debug!("probe-other: tree is={is:?} under={under:?} foreign={foreign:?}");

    // 四、判据：**一例一条**——三格都恰是 `DENIED`（不是放行，也不是"判不了"）。
    {
        assert_eq!(is, Err(Fail::Denied))
    }
    {
        assert_eq!(under, Err(Fail::Denied))
    }
    {
        assert_eq!(foreign, Err(Fail::Denied))
    }

    let complete = protocol::communication::session::establish::claim(sire, env::Mark::of("probe-rule-verified"), Wait::AtMost(MS)).expect("probe-other: completion channel");
    runtime::env::mail::HolePie::from_token(complete).push(&[1], Wait::AtMost(MS)).expect("probe-other: completion reply");
    return Report::note(E_OK, OK_NOTE);
}

/// 沿一条路译成号再 `find`：`Ok(())` = 放行；答不出 / 门禁答"不"落 Fail
/// **（两格为什么分开写）**：新面把这两件分在
/// 而这一台量的正是门禁那一格（`Denied`），故两格各写一次，读的人一眼看得见"拒"是从哪一问来的
/// 重试那一层
fn denied(tree: &TreeFace, base: &Path, leaf: &str) -> Result<(), Fail> {
    let road = base.try_join(leaf).ok_or(Fail::Unknown)?;
    let root = tree.root();
    let mut left = MS;
    loop {
        match root
            .tile(&road, Wait::AtMost(MS))
            .and_then(|entry| entry.token(Wait::AtMost(MS)))
            .map(|_| ())
        {
            Ok(()) => return Ok(()),
            Err(Fail::Unknown) if left > 0 => {
                let _ = runtime::env::room::sleep(core::time::Duration::from_millis(1));
                left = left.saturating_sub(1);
            }
            Err(fail) => return Err(fail),
        }
    }
}

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}
