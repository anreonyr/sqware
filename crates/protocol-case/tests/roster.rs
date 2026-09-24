//! 名册与谱系（+ 盟籍）的门（**宿主台**）—— 三本册子的规矩，在宿主上真跑一遍。
//!
//! # 这一台为什么存在（照实记：这一批是"救活的"）
//!
//! `crates/contract/src/system/principal/core.rs` 与 `crates/contract/src/system/coalition/core.rs` 各自的
//! `#[cfg(test)]` 模块**从写下那天起一次没跑过**：`protocol` 是 `[lib] test = false`
//! （riscv 目标上编不出 libtest），而主工作区那几道门（`check --all-targets` /
//! `build --release`）一道都不编它——那批规格长期只有"写着的规格"、没有"跑着的判据"。
//!
//! 与别的几台同一条路（清单见 `crates/gate/tests/host.rs` 头注；头几台是 `operator` 靶 /
//! `line` 靶 / `judge` 靶）：编外宿主
//! crate、只依赖 `env`、把核心源码**逐字未改**地 `#[path]` 进来，门口 `crates/gate/tests/host.rs`。
//!
//! **两本册子同住一台**：盟籍核心写着 `use crate::system::principal::core::PrincipalId` —— 它要身份
//! 那本册子的号。分两台各编一遍的话，`principal/core.rs` 里那批判据会在两个靶里各跑一遍
//! （`judge` 靶的头注记过同一条）。故同住一台。
//!
//! **照实记（这一台的文件名）**：靶子的根文件叫 `roster.rs` 而不是 `principal.rs`——因为
//! 它要给 `crate::system::principal::core` 一个**真实的目录模块**（`tests/principal/mod.rs`），
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
#[path = "../../contract/src/fail_codes.rs"]
mod fail_codes;

/// 号的词汇（`crates/contract/src/id.rs`，逐字未改）——三个号空间共用的一条规则与那 8 字节。
/// 两本册子与它们的帧都写着 `use crate::id::Id`，故这一台要给它那个名字。
#[path = "../../contract/src/id.rs"]
mod id;

/// 定长一问一答的**帧骨架**（`crates/contract/src/frame.rs`，逐字未改）——principal 与 coalition
/// 同形的那一份（长度、编 / 解、答话那几手）。两族的 `frame.rs` 都写着 `use crate::frame::…`。
#[path = "../../contract/src/frame.rs"]
mod frame;

/// 身份那本册子（就是 `crates/contract/src/system/principal/core.rs` 那一份，逐字未改）。
///
/// 包一层目录模块（`tests/principal/`）只为让 `crate::system::principal::core` 这个名字成立
/// ——盟籍那一份正是这么写它的 `use`（在 `protocol` 里它是 `crate::system::principal::core`，
/// 这里逐字同形）。
mod principal;

/// 盟籍那两片（`crates/protocol/src/system/coalition/{core,frame}.rs`，逐字未改）住在
/// `tests/coalition/` 那个**目录模块**里——帧那一份写的是 `use super::core::…`，故两片必须同层。
mod coalition;

/// 搬进 `system/` 之后（用户裁定），被**逐字**编进来的那两份源码里写的是
/// `crate::system::{principal, coalition}`；而靶自己的模块树仍按"要哪几片编哪几片"摆在根上。
/// 故这里补一个**只做转出的桩**让那个名字成立——不是再挖一层目录：`#[path]` 的基准会跟着变，
/// 那个坑记在 `tests/principal/mod.rs`。
mod system {
    pub(crate) use crate::{coalition, principal};
}

// ── 帧那一半（`principal/frame.rs` 与 `coalition/frame.rs`）──────────
//
// **照实记（这一组为什么值当）**：机器那几道门走的是**顺路**——客侧编一帧、服务侧解一帧，
// 形状对了就继续。下面这些格子机器**一条都走不到**：短帧 / 长帧 / 动作码那一格读不懂、
// **游标那一格的双射**（`0` = 没有游标，而**零号是真格子**）、窗答的条数与帧长对不对得上、
// 以及失败码表的两端。

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
    assert_eq!(
        pframe::unpack_ask(&frame[..pframe::ASK_LEN - 1]),
        None,
        "短一字节"
    );
    assert_eq!(
        pframe::op_of(&frame[..1]),
        Some(pframe::HEIR),
        "动作码那一格读得出"
    );
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
    assert_eq!(
        cframe::cursor_in(1),
        Some(0),
        "解回来是零号——**不是「没有」**"
    );
    assert_eq!(cframe::cursor_in(10), Some(9));
}

#[test]
fn a_coalition_window_of_numbers_round_trips_with_its_count_and_more_flag() {
    let window: Window<PrincipalId> = Window::gather(
        true,
        [
            PrincipalId::new(2),
            PrincipalId::new(0),
            PrincipalId::new(5),
        ]
        .into_iter(),
    );
    let mut out = [0u8; cframe::REPLY_MAX];
    let n = cframe::pack_seq(&mut out, &window);
    assert_eq!(n, 3 + 3 * 8, "帧长 = 3 + 8 × 枚数");
    assert_eq!(out[1], 1, "未那一格");
    assert_eq!(out[2], 3, "条数那一格");

    let back: Window<PrincipalId> = cframe::read_seq(&out[..n]).expect("读得回来");
    assert_eq!(back.len(), 3);
    assert!(back.more(), "窗外还有");
    assert_eq!(
        back.iter().collect::<Vec<_>>(),
        window.iter().collect::<Vec<_>>()
    );

    // **这一族不猜**：帧长与条数对不上就是读不懂。
    assert_eq!(
        cframe::read_seq::<PrincipalId>(&out[..n - 1]),
        Err(cframe::BAD),
        "短一字节"
    );
    assert_eq!(
        cframe::read_seq::<PrincipalId>(&out[..n + 1]),
        Err(cframe::BAD),
        "多一字节"
    );
    let mut liar = out;
    liar[2] = 4; // 说有四枚，可帧里只有三枚
    assert_eq!(
        cframe::read_seq::<PrincipalId>(&liar[..n]),
        Err(cframe::BAD),
        "条数说谎"
    );
}

#[test]
fn a_coalition_answer_has_three_shapes_and_they_all_read_back() {
    // 三形：一格状态码 / 一枚号 / 一个是非。
    let status = cframe::reply_status(cframe::UNKNOWN);
    assert_eq!(cframe::unpack_reply(&status), Some((cframe::UNKNOWN, 0, 0)));

    // `FOUND` 那一形**不用 `flag`**：它必有号（零号也是合法答案），没有"没有"这一档。
    let value = cframe::reply_value(CoalitionId::new(12));
    assert_eq!(
        cframe::unpack_reply(&value),
        Some((cframe::OK, 0, 12)),
        "号在 `a` 那一格"
    );
    let zero = cframe::reply_value(CoalitionId::new(0));
    assert_eq!(
        cframe::unpack_reply(&zero),
        Some((cframe::OK, 0, 0)),
        "零号也读得回来"
    );

    // 而 `AMID` 那一形：是非在 `flag` 那一格，`a` 那一格空着。
    let yes = cframe::reply_yes(true);
    assert_eq!(cframe::unpack_reply(&yes), Some((cframe::OK, 1, 0)));
    let no = cframe::reply_yes(false);
    assert_eq!(cframe::unpack_reply(&no), Some((cframe::OK, 0, 0)));
    assert_ne!(yes, no, "是非那一格分得开");

    assert_eq!(cframe::unpack_reply(&[]), None, "空帧");
    assert_eq!(
        cframe::unpack_reply(&yes[..cframe::REPLY_LEN - 1]),
        None,
        "短一字节"
    );
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

// ── 面不相撞那一条用例搬去了**编译期**（用户裁定"常量交给编译器"）────────────
//
// `the_three_back_marks_of_the_three_doors_do_not_collide` 原先在这里，那几条现在写在
// `crates/contract/src/system/principal/frame.rs` 与 `coalition/frame.rs` 的
// `const _: () = assert!(…)` 里（跨门那一对钉在前者——它看得见 `crate::system::coalition`）。
// ── 名册与谱系那本册子的判据（原住 `principal/core.rs` 的 `#[cfg(test)]`）────────────
//
// **照实记（用户裁定）**："我希望测试和运行环境分开，而不是交叉在一起" ⇒ 那 **13 条**用例不再
// 住运行时源里，整段搬到这里（两份核心各自留下结论与去处）。
//
// 两个 `mod`：两份正文里都有 `A` / `book()`，同住一层会撞；各自 `use` 自己那本册子的东西。

mod principal_core {
    use crate::principal::core::*;
    use env::TaskId;

    const A: TaskId = TaskId::new(11); // 装配者
    const ME: TaskId = TaskId::new(22); // 一条被别人绑的 TID

    fn book() -> Principal {
        Principal::new(A).expect("根立得起来")
    }

    #[test]
    fn the_root_has_no_sire_and_a_tree_outside_id_is_unknown() {
        let b = book();
        assert_eq!(b.sire(PrincipalId::ROOT), Ok(None));
        assert_eq!(b.sire(PrincipalId::new(4095)), Err(Fail::Unknown));
    }

    #[test]
    fn a_bound_task_resolves_and_rebinding_replaces() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).expect("装配者能派生");
        assert_eq!(b.resolve(ME), None);
        b.bind(A, ME, p).expect("装配者能绑");
        assert_eq!(b.resolve(ME), Some(p));
        let q = b.derive(A, PrincipalId::ROOT).unwrap();
        b.bind(A, ME, q).unwrap();
        assert_eq!(b.resolve(ME), Some(q));
    }

    #[test]
    fn only_the_assembler_writes_the_roster() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        assert_eq!(b.bind(ME, ME, p), Err(Fail::Denied));
        assert_eq!(b.unbind(ME, ME), Err(Fail::Denied));
        assert_eq!(b.unbind(A, ME), Err(Fail::Unknown));
    }

    #[test]
    fn a_representative_derives_downwards_only() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        b.bind(A, ME, p).unwrap();
        let q = b.derive(ME, p).expect("代表 p 的那一枚能向下派生");
        assert_eq!(b.sire(q), Ok(Some(p)));
        // 换绑到 q 之后，同一枚不能再回头派生 p 的另一个孩子（钥匙是"正好代表 p"）。
        b.bind(A, ME, q).unwrap();
        assert_eq!(b.derive(ME, p), Err(Fail::Denied));
    }

    #[test]
    fn heir_is_reflexive_asymmetric_and_three_state() {
        let mut b = book();
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        let q = b.derive(A, p).unwrap();
        assert_eq!(b.heir(p, p), Ok(true));
        assert_eq!(b.heir(p, q), Ok(true));
        assert_eq!(b.heir(q, p), Ok(false));
        assert_eq!(b.heir(PrincipalId::new(4095), p), Err(Fail::Unknown));
    }

    #[test]
    fn clan_meets_at_the_nearest_common_ancestor() {
        let mut b = book();
        let x = b.derive(A, PrincipalId::ROOT).unwrap();
        let y = b.derive(A, PrincipalId::ROOT).unwrap();
        let z = b.derive(A, y).unwrap();
        assert_eq!(b.clan(x, z), Ok(PrincipalId::ROOT));
        assert_eq!(b.clan(y, z), Ok(y));
        assert_eq!(b.clan(z, z), Ok(z));
    }

    #[test]
    fn conversion_keeps_to_its_own_branch_and_waive_returns_to_origin() {
        let mut b = book();
        // 装配给 ME 的起点是 p；p 下再派生 q；另有一支 r 不在 p 下面。
        let p = b.derive(A, PrincipalId::ROOT).unwrap();
        b.bind(A, ME, p).unwrap();
        let q = b.derive(A, p).unwrap();
        let r = b.derive(A, PrincipalId::ROOT).unwrap();

        assert_eq!(b.adopt(ME, q), Ok(()));
        assert_eq!(b.resolve(ME), Some(q));
        // **照实记（搬出运行时源时改的一处）**：这里原先是
        // `b.heir(b.roster[0].origin, b.roster[0].current)`——读的是 `Principal` 的**私有
        // 字段**，在源里才够得着。搬出来之后那条不变量换个可观察的读法，就在下面：
        // `waive` 回的是**起点**（`Some(p)`），而此刻 `current` 已经是 `q`。
        // **钥匙反证**：已不代表 p，故"从 p 派生"被拒——转换是真的。
        assert_eq!(b.derive(ME, p), Err(Fail::Denied));
        // 向上（p 是 q 的父，不在 q 那一支里）与跨支都不许。
        assert_eq!(b.adopt(ME, p), Err(Fail::Denied));
        assert_eq!(b.adopt(ME, r), Err(Fail::Denied));
        // 树外。
        assert_eq!(b.adopt(ME, PrincipalId::new(4095)), Err(Fail::Unknown));
        // 弃 = 回到起点；**不删格**，故还能再领一次。
        assert_eq!(b.waive(ME), Ok(()));
        assert_eq!(b.resolve(ME), Some(p));
        assert_eq!(b.adopt(ME, q), Ok(()));
        // 换绑 = 重定起点：弃回的是**新**起点（两格一起写）。
        b.bind(A, ME, r).unwrap();
        assert_eq!(b.resolve(ME), Some(r));
        assert_eq!(b.waive(ME), Ok(()));
        assert_eq!(b.resolve(ME), Some(r));

        // 没绑过的那一枚：两条转换都答 Unknown（与 unbind 撞空同调）。
        let other = TaskId::new(33);
        assert_eq!(b.adopt(other, p), Err(Fail::Unknown));
        assert_eq!(b.waive(other), Err(Fail::Unknown));
    }
}

// ── 盟籍那本册子的判据（原住 `coalition/core.rs` 的 `#[cfg(test)]`）────────────

mod coalition_core {
    use crate::coalition::core::*;
    use crate::principal::core::PrincipalId;

    fn book() -> Coalition {
        Coalition::new()
    }

    const A: PrincipalId = PrincipalId::new(11);
    const B: PrincipalId = PrincipalId::new(22);
    /// 伪造的线上值：铸过的号是 `0..next`，故这个一定在册外。
    const OUTSIDE: CoalitionId = CoalitionId::new(4095);

    #[test]
    fn found_mints_dense_monotone_numbers_and_never_fails() {
        let mut b = book();
        assert_eq!(b.found(), CoalitionId::new(0));
        assert_eq!(b.found(), CoalitionId::new(1));
        // 空盟合法：铸出来一枚都没进，也没有"散掉"这回事。
        assert_eq!(b.amid(A, CoalitionId::new(0)), Ok(false));
    }

    #[test]
    fn a_coalition_outside_the_counter_is_unknown_for_writes_and_band() {
        let mut b = book();
        assert_eq!(b.enter(A, OUTSIDE), Err(Fail::Unknown));
        assert_eq!(b.leave(A, OUTSIDE), Err(Fail::Unknown));
        assert_eq!(b.amid(A, OUTSIDE), Err(Fail::Unknown));
        assert!(b.band(OUTSIDE, None).is_err());
    }

    #[test]
    fn enter_and_leave_are_idempotent_and_bloc_has_no_failure() {
        let mut b = book();
        let c = b.found();
        assert_eq!(b.enter(A, c), Ok(()));
        assert_eq!(b.enter(A, c), Ok(())); // 第二次：表不动
        assert_eq!(b.amid(A, c), Ok(true));
        // **"表不动"看见的样子**：四条读（`amid` / `band` / `bloc` / `leave`）里，同一对
        // 都只算一格。照实记：这一格是牙口量出来的，结论有点反直觉——把 `enter` 里那次
        // 查重删掉（同一对真的进两行），**这四条读全都看不出来**：`amid` 照样真、
        // 两趟取窗本来就按键去重（`window` 挑"比上一枚大的里头最小的"）、`leave` 把两行
        // 一起拿掉。故那一行是**卫生**（不让表长冗余），不是语义——这一格钉的是"读出来是一格"。
        assert_eq!(
            b.band(c, None).expect("铸过的盟").len(),
            1,
            "同一对只许一格"
        );
        assert_eq!(b.bloc(A, None).len(), 1, "反向那一次也一样");
        assert_eq!(b.leave(A, c), Ok(()));
        assert_eq!(b.leave(A, c), Ok(())); // 撞空也成
        assert_eq!(b.amid(A, c), Ok(false));
        // 假身份：不在任何盟里 ⇒ 空串（**不是** Unknown——本册不去问身份服务）。
        assert_eq!(b.bloc(PrincipalId::new(4095), None).len(), 0);
    }

    #[test]
    fn one_a_pair_is_one_row_and_two_identities_can_share_a_coalition() {
        let mut b = book();
        let c0 = b.found();
        let c1 = b.found();
        b.enter(A, c0).unwrap();
        b.enter(B, c0).unwrap();
        b.enter(A, c1).unwrap();
        assert_eq!(
            b.band(c0, None).unwrap().iter().collect::<Vec<_>>(),
            alloc::vec![A, B]
        );
        assert_eq!(
            b.band(c1, None).unwrap().iter().collect::<Vec<_>>(),
            alloc::vec![A]
        );
        assert_eq!(
            b.bloc(A, None).iter().collect::<Vec<_>>(),
            alloc::vec![c0, c1]
        );
        // 出去的是"这一对"，不是"这个人"。
        b.leave(A, c0).unwrap();
        assert_eq!(
            b.band(c0, None).unwrap().iter().collect::<Vec<_>>(),
            alloc::vec![B]
        );
        assert_eq!(b.bloc(A, None).iter().collect::<Vec<_>>(), alloc::vec![c1]);
    }

    #[test]
    fn a_window_is_number_ordered_and_the_cursor_is_a_threshold() {
        let mut b = book();
        let c = b.found();
        // 登记序是 B 再 A，号序是 A(11) 再 B(22)——读数按号排，不按登记排。
        b.enter(B, c).unwrap();
        b.enter(A, c).unwrap();
        let all = b.band(c, None).unwrap();
        assert_eq!(all.iter().collect::<Vec<_>>(), alloc::vec![A, B]);
        assert!(!all.more());
        // 阈值：取号 > A 的那些 ⇒ 只剩 B
        assert_eq!(
            b.band(c, Some(A)).unwrap().iter().collect::<Vec<_>>(),
            alloc::vec![B]
        );
        // **阈值大于一切 ⇒ 空窗，不是错**（"过期游标"在阈值语义下不存在）
        // （"空"用 `len` 问——`Window` 的读面只留 `len` / `more` / `get` / `iter`，见它的注。）
        assert_eq!(b.band(c, Some(PrincipalId::new(4095))).unwrap().len(), 0);
        // 零号是真格子：从最小那一头数起，`PrincipalId::ROOT`(0) 那一位也要数得到
        b.enter(PrincipalId::ROOT, c).unwrap();
        assert_eq!(
            b.band(c, None).unwrap().iter().collect::<Vec<_>>(),
            alloc::vec![PrincipalId::ROOT, A, B]
        );
    }

    #[test]
    fn a_full_window_says_there_is_more_and_the_last_one_is_the_next_cursor() {
        let mut b = book();
        let c = b.found();
        for i in 0..=WINDOW_CAP {
            b.enter(PrincipalId::new(i), c).unwrap();
        }
        let first = b.band(c, None).unwrap();
        assert_eq!(first.len(), WINDOW_CAP);
        assert!(first.more());
        // 拿末一枚当阈值接着取：剩下就是窗外那一条
        let next = b.band(c, first.last()).unwrap();
        assert_eq!(next.len(), 1);
        assert!(!next.more());
        assert_eq!(next.get(0), Some(PrincipalId::new(WINDOW_CAP)));
    }
}
