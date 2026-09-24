//! 名册与谱系（+ 盟籍）的门（**宿主台**）—— 三本册子的规矩，在宿主上真跑一遍。
//!
//! # 这一台为什么存在（照实记：这一批是"救活的"）
//!
//! `crates/protocol/src/principal/core.rs` 与 `crates/protocol/src/coalition/core.rs` 各自的
//! `#[cfg(test)]` 模块**从写下那天起一次没跑过**：`protocol` 是 `[lib] test = false`
//! （riscv 目标上编不出 libtest），而主工作区那几道门（`check --all-targets` /
//! `build --release`）一道都不编它——那批规格长期只有"写着的规格"、没有"跑着的判据"。
//!
//! 与别的几台同一条路（清单见 `scripts/host.sh` 头注；头几台是 `operator-case` /
//! `line-case` / `judge-case`）：编外宿主
//! crate、只依赖 `env`、把核心源码**逐字未改**地 `#[path]` 进来，门口 `scripts/host.sh`。
//!
//! **两本册子同住一台**：盟籍核心写着 `use crate::principal::core::PrincipalId` —— 它要身份
//! 那本册子的号。分两台各编一遍的话，`principal/core.rs` 里那批判据会在两个靶里各跑一遍
//! （`judge-case` 的头注记过同一条）。故同住一台。
//!
//! **照实记（这一台的文件名）**：靶子的根文件叫 `roster.rs` 而不是 `principal.rs`——因为
//! 它要给 `crate::principal::core` 一个**真实的目录模块**（`tests/principal/mod.rs`），
//! 而 `tests/principal.rs` 与 `tests/principal/` 同名会撞（E0761）。
//!
//! # 这一台钉的是什么
//!
//! **名册**（TID → 此刻代表的号）：一 TID 一格、只有装配者写得动、换绑 = 重定起点。
//! **谱系**（号 → 父）：只增不删、下标即号、零号是根、恰好一个根；`heir` 自反且反对称。
//! **转换**（`adopt` / `waive`）：身份只沿自己那一支往下走，或者回到起点（`origin ≼ current`）。
//! **盟籍**（一张两列表）：反着念是同一个关系的两个方向；空盟合法、号铸过就一直在；
//! `enter` / `leave` 幂等；取窗是"号序 + 阈值游标"。

extern crate alloc;

/// 码表宏（`fail_codes!`）自己一份源——**协议与宿主靶同读这一份**。
///
/// 两样都要：`#[macro_use]` 把宏带进**下面那些模块**的作用域（宏的可见性按正文先后 ⇒ 这一行
/// 必须在帧模块之前），`#[macro_export]` 保住"出 crate"那一份。见那份文件的照实记。
#[macro_use]
#[path = "../../protocol/src/fail_codes.rs"]
mod fail_codes;

/// 身份那本册子（就是 `crates/protocol/src/principal/core.rs` 那一份，逐字未改）。
///
/// 包一层内联模块只为让 `crate::principal::core` 这个名字成立——盟籍那一份正是这么写它的
/// `use`（在 `protocol` 里它是 `crate::principal::core`，这里逐字同形）。
mod principal;

/// 盟籍那两片（`crates/protocol/src/coalition/{core,frame}.rs`，逐字未改）住在
/// `tests/coalition/` 那个**目录模块**里——帧那一份写的是 `use super::core::…`，故两片必须同层。
mod coalition;

// ── 帧那一半（`principal/frame.rs` 与 `coalition/frame.rs`）──────────
//
// **照实记（这一组为什么值当）**：机器那几道门走的是**顺路**——客侧编一帧、服务侧解一帧，
// 形状对了就继续。下面这些格子机器**一条都走不到**：短帧 / 长帧 / 动作码那一格读不懂、
// **游标那一格的双射**（`0` = 没有游标，而**零号是真格子**）、窗答的条数与帧长对不对得上、
// 以及失败码表的两端。

use env::Mark;
use coalition::core::{CoalitionId, Fail as CFail, Window};
use coalition::frame as cframe;
use principal::core::{Fail as PFail, PrincipalId};
use principal::frame as pframe;

#[test]
fn a_principal_ask_round_trips_and_refuses_other_shapes() {
    let frame = pframe::pack_ask(pframe::HEIR, 7, 9);
    assert_eq!(frame.len(), pframe::ASK_LEN);
    assert_eq!(pframe::op_of(&frame), Some(pframe::HEIR));
    assert_eq!(pframe::unpack_ask(&frame), Some((pframe::HEIR, 7, 9)));

    // 不成形就不猜。
    assert_eq!(pframe::unpack_ask(&[]), None, "空帧");
    assert_eq!(pframe::unpack_ask(&frame[..pframe::ASK_LEN - 1]), None, "短一字节");
    assert_eq!(pframe::op_of(&frame[..1]), Some(pframe::HEIR), "动作码那一格读得出");
    assert_eq!(pframe::op_of(&[]), None, "空帧连动作码都没有");
    let long = [frame.as_slice(), &[0u8]].concat();
    assert_eq!(pframe::unpack_ask(&long), None, "长一字节");
}

#[test]
fn a_principal_answer_is_not_a_failure_so_it_travels_as_ok_plus_a_flag() {
    // **答案不进失败表**（文件头那一段）：`RESOLVE` 的"没绑"与 `SIRE` 的"它是根"都是诚实的
    // 答案 ⇒ 走 `status == OK` + `flag`；而 **`PrincipalId(0)` 是根，不是"没有"**——故 `a`
    // 那一格里的 0 是合法答案，"有没有"只能另占一格（`flag`）。
    let absent = pframe::reply_present(false, PrincipalId::new(0));
    assert_eq!(absent[0], pframe::OK, "没绑是答案，不是失败");
    assert_eq!(absent[1], 0, "flag = 没有");
    assert_eq!(pframe::unpack_reply(&absent), Some((pframe::OK, 0, 0)));

    let root = pframe::reply_present(true, PrincipalId::ROOT);
    assert_eq!(root[1], 1, "flag = 有");
    assert_eq!(
        pframe::unpack_reply(&root),
        Some((pframe::OK, 1, 0)),
        "**根（号 0）是一个合法答案**，读得回来"
    );

    // 「没有」与「号 0」分得开：两帧形状不同。
    assert_ne!(absent, root, "`flag` 那一格不能省");
}

#[test]
fn the_principal_failure_table_is_bijective_and_keeps_bad_outside() {
    use pframe::{BAD, DENIED, NO_ROOM, OK, UNKNOWN, code_to_fail, fail_to_code};
    assert_eq!(fail_to_code(None), OK);
    assert_eq!(fail_to_code(Some(PFail::Denied)), DENIED);
    assert_eq!(fail_to_code(Some(PFail::Unknown)), UNKNOWN);
    assert_eq!(fail_to_code(Some(PFail::NoRoom)), NO_ROOM);
    assert_eq!(code_to_fail(OK), None);
    assert_eq!(code_to_fail(DENIED), Some(PFail::Denied));
    assert_eq!(code_to_fail(UNKNOWN), Some(PFail::Unknown));
    assert_eq!(code_to_fail(NO_ROOM), Some(PFail::NoRoom));
    assert_eq!(code_to_fail(BAD), None, "读不懂那一格在失败域之外");
    assert_eq!(code_to_fail(99), None, "表外的码");
    assert_ne!(BAD, OK);
}

#[test]
fn a_coalition_ask_round_trips_and_the_cursor_cell_is_a_bijection() {
    let frame = cframe::pack_ask(cframe::BAND, 3, 0);
    assert_eq!(cframe::unpack_ask(&frame), Some((cframe::BAND, 3, 0)));
    assert_eq!(cframe::op_of(&frame), Some(cframe::BAND));
    assert_eq!(cframe::unpack_ask(&frame[..cframe::ASK_LEN - 1]), None);

    // **游标那一格加一**（双射）：零号是真格子（`PrincipalId::ROOT` 是 0），拿 0 当"没有"
    // 会把它漏掉——故 `0` = 没有游标，`号 + 1` = 从那一号之后接着取。
    assert_eq!(cframe::cursor_of::<PrincipalId>(None), 0);
    assert_eq!(cframe::cursor_of(Some(PrincipalId::new(0))), 1, "零号 + 1");
    assert_eq!(cframe::cursor_of(Some(PrincipalId::new(9))), 10);
    assert_eq!(cframe::cursor_in(0), None, "0 = 从头取");
    assert_eq!(cframe::cursor_in(1), Some(0), "解回来是零号——**不是「没有」**");
    assert_eq!(cframe::cursor_in(10), Some(9));
}

#[test]
fn a_coalition_window_of_numbers_round_trips_with_its_count_and_more_flag() {
    let window: Window<PrincipalId> = Window::gather(
        true,
        [PrincipalId::new(2), PrincipalId::new(0), PrincipalId::new(5)].into_iter(),
    );
    let mut out = [0u8; cframe::REPLY_MAX];
    let n = cframe::pack_seq(&mut out, &window);
    assert_eq!(n, 3 + 3 * 8, "帧长 = 3 + 8 × 枚数");
    assert_eq!(out[1], 1, "未那一格");
    assert_eq!(out[2], 3, "条数那一格");

    let back: Window<PrincipalId> = cframe::read_seq(&out[..n]).expect("读得回来");
    assert_eq!(back.len(), 3);
    assert!(back.more(), "窗外还有");
    assert_eq!(back.iter().collect::<Vec<_>>(), window.iter().collect::<Vec<_>>());

    // **这一族不猜**：帧长与条数对不上就是读不懂。
    assert_eq!(cframe::read_seq::<PrincipalId>(&out[..n - 1]), Err(cframe::BAD), "短一字节");
    assert_eq!(cframe::read_seq::<PrincipalId>(&out[..n + 1]), Err(cframe::BAD), "多一字节");
    let mut liar = out;
    liar[2] = 4; // 说有四枚，可帧里只有三枚
    assert_eq!(cframe::read_seq::<PrincipalId>(&liar[..n]), Err(cframe::BAD), "条数说谎");
}

#[test]
fn a_coalition_answer_has_three_shapes_and_they_all_read_back() {
    // 三形：一格状态码 / 一枚号 / 一个是非。
    let status = cframe::reply_status(cframe::UNKNOWN);
    assert_eq!(cframe::unpack_reply(&status), Some((cframe::UNKNOWN, 0, 0)));

    // `FOUND` 那一形**不用 `flag`**：它必有号（零号也是合法答案），没有"没有"这一档。
    let value = cframe::reply_value(CoalitionId::new(12));
    assert_eq!(cframe::unpack_reply(&value), Some((cframe::OK, 0, 12)), "号在 `a` 那一格");
    let zero = cframe::reply_value(CoalitionId::new(0));
    assert_eq!(cframe::unpack_reply(&zero), Some((cframe::OK, 0, 0)), "零号也读得回来");

    // 而 `AMID` 那一形：是非在 `flag` 那一格，`a` 那一格空着。
    let yes = cframe::reply_yes(true);
    assert_eq!(cframe::unpack_reply(&yes), Some((cframe::OK, 1, 0)));
    let no = cframe::reply_yes(false);
    assert_eq!(cframe::unpack_reply(&no), Some((cframe::OK, 0, 0)));
    assert_ne!(yes, no, "是非那一格分得开");

    assert_eq!(cframe::unpack_reply(&[]), None, "空帧");
    assert_eq!(cframe::unpack_reply(&yes[..cframe::REPLY_LEN - 1]), None, "短一字节");
}

#[test]
fn the_coalition_failure_table_is_bijective_and_keeps_bad_outside() {
    use cframe::{BAD, NO_ROOM, OK, UNKNOWN, code_to_fail, fail_to_code};
    assert_eq!(fail_to_code(None), OK);
    assert_eq!(fail_to_code(Some(CFail::Unknown)), UNKNOWN);
    assert_eq!(fail_to_code(Some(CFail::NoRoom)), NO_ROOM);
    assert_eq!(code_to_fail(OK), None);
    assert_eq!(code_to_fail(UNKNOWN), Some(CFail::Unknown));
    assert_eq!(code_to_fail(NO_ROOM), Some(CFail::NoRoom));
    assert_eq!(code_to_fail(BAD), None, "读不懂那一格在失败域之外");
    assert_ne!(BAD, OK);
}

#[test]
fn the_three_back_marks_of_the_three_doors_do_not_collide() {
    // **面不相撞**：三条路各自的回信孔记号互不相同（同一张表里分得出这一枚是哪一面的）。
    use cframe as c;
    use pframe as p;
    let marks = [p::BACK, c::BACK, Mark::of("board-back")];
    assert_ne!(p::BACK, c::BACK);
    assert_ne!(p::BACK, Mark::of("board-back"));
    assert_ne!(c::BACK, Mark::of("board-back"));
    for m in marks {
        assert_ne!(m, Mark::NONE);
    }
    // 也不能与"那一条路的名字"撞（`Mark::of(NAME)` 是同一张表里的另一枚）。
    assert_ne!(p::BACK, Mark::of(p::NAME));
    assert_ne!(c::BACK, Mark::of(c::NAME));
    assert_eq!(p::DIR, "sys");
    assert_eq!(p::NAME, "principal");
    assert_eq!(c::NAME, "coalition");
}

