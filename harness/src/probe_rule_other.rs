#![no_std]
#![no_main]

//! probe-rule-other — **另一位客人**：**有身份**地去用别人立了规矩的那几格，期望被拒。
//!
//! `probe-rule` 那一台证的是"**规矩随身份走**"（同一个 TID 换一位代表，答案就变了）。
//! 而 `Permit::Trunk` 与 `Permit::Bough` 各还有一格**只有另一台客人量得到**：
//!
//! - `Bough(p)` 的负证要一位**不在 p 那一支里**的——同一台客人做不到：`q = derive(p)` 一定
//!   在 p 那一支里，而 `adopt` 只许**往下**领（`heir(current, q)`，见
//!   `protocol::system::principal::core` 的 `Principal::adopt` 三格前置）；
//! - `Trunk(p)` 的负证要一位**不是 p** 的——`probe-rule` 用 adopt 演过一次，本台再换**一台客人**
//!   演一次：装配期每位都是 `derive(ROOT)` 的**兄弟**，故彼此都不在对方那一支里。
//!
//! ```text
//!   1  上树、开一条问话孔（本域不需要门牌：只 `seek` + `find`，不问身份服务）
//!   2  SEEK /svc/rule/is      ⇒ FIND ⇒ 期望 DENIED(8)
//!   3  SEEK /svc/rule/under   ⇒ FIND ⇒ 期望 DENIED(8)
//!   4  SEEK /svc/rule/foreign ⇒ FIND ⇒ 期望 DENIED(8)
//!      —— 那一格许给的是"**开着 `/svc/sys/principal/ask` 那一格**的那位"（规矩由 `probe-rule` 落，
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
// ——与 `canonical` / `probe-denied` 同一条：`programs/src/user/mod.rs` 里没有它。
//
// 两条 `extern crate` 缺一不可（实测）：`alloc` 是 `format!` 要用；`programs` **不是**为了
// 用它里面的东西，而是为了把 `libprograms` 链进来——**panic handler 与 `_start` 都住那份
// lib**（`programs/src/entry.rs`）。少了它，链接期报 `` `#[panic_handler]` function required ``。
extern crate alloc;
extern crate programs;

use env::Wait;
use programs::Report;

use protocol::communication::session::Session;
use protocol::debug;
use protocol::system::operator::Fail;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Face as TreeFace;
use protocol::system::operator::path::Path;
use runtime::env::unit as utask;

/// **容器那一段那一条路**（`/svc`）——那一段名字只在协议那一侧说（见 `probe_lease` 同款）。
const DIR: protocol::system::operator::Path = protocol::system::SVC;
const PANE: &str = "rule";
const IS: &str = "is";
const UNDER: &str = "under";
/// `probe-rule` 落的第三格：规矩 = `Opener(/svc/sys/principal/ask 那一格)`（许给**别人**）。
const FOREIGN: &str = "foreign";

/// 等树 / 等答的总上限（毫秒）。**必须有界**：对面死在头几步时本域不能陪着挂死。
const MS: usize = 1000;

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
    let sire = utask::sire();

    // 一、上树：本域只开一条会话（不找门牌——本台只 `seek` / `find`，不问身份）。
    //
    // **照实记（这一处为什么包成 `Face`，task-2 那一刀）**：那条会话上本域只要"名字 → 入口"
    // 一条路 ⇒ 交给 [`TreeFace::of`]（吃所有权），下面三问从"两个参数"变成"一条路 + 一份期限"。
    let Ok(session) = Session::open(sire, operator::BERTH, Wait::AtMost(MS)) else {
        return bail("probe-other: no tree link");
    };
    let tree = TreeFace::of(session);
    // 本台那三格都挂在本域那一块下面（`/svc/rule`）——故先拼出那一条路。
    let base = DIR.join(PANE);

    // 二、按名字取号（**这一手不过门禁**：`seek` 不在闸口里），再 `find`——那几手该被拒。
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

    return Report::note(E_OK, OK_NOTE);
}

/// 沿一条路译成号再 `find`：`Ok(())` = 放行；答不出 / 门禁答"不"落 [`Fail`]。
///
/// **译不出就重试**（有界）：`/svc/rule` 那几格由另一台客人落下，它可能落得比本域晚。
///
/// **照实记（收 `&TreeFace`，不再收 `&Session`）**：调用方**已持**一面（task-2 那一刀包出来的）。
///
/// **照实记（两格为什么分开写）**：旧面 `entry_of` = 译号（重试）＋一趟 `find`。新面把这两件分在
/// 两个柄上：`Pane::tile` 译号、[`Tile::token`] 取那一枚——**门禁那一趟发生在后面的 `find`**，
/// 而这一台量的正是门禁那一格（`Denied`），故两格各写一次，读的人一眼看得见"拒"是从哪一问来的。
///
/// **照实记（重试在这一格里）**：`Pane::tile` 自己**不带重试**（就地问一次）；"译不出就再问"
/// 由下面这一圈承担。要带额度的那一形是 [`Face::tile`]——它同样只译号（不飞门闩），差别只有
/// 重试那一层。
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

/// 哪里算不下去就报哪一句（kernel 收场时把这一句连同域号打出来）。
fn bail<'a>(note: &'a str) -> Report<'a> {
    debug!("{}", note);
    return Report::note(E_TRIP, note);
}
