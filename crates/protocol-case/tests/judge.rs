//! 门外那一问的门（**宿主台**）—— 裁决那三格规矩 + 它翻成线上那一格 + **那一本账**，
//! 在宿主上真跑一遍。
//!
//! # 这批判据钉的是什么
//!
//! `crates/contract/src/system/operator/judge.rs` 是"这一位许不许动这一格"的**定义**；
//! `gate.rs` 是它**翻成线上那一格**的那一层；`ledger.rs` 是"这一格的规矩与归属"**记在哪**。
//! 三份都一行不发消息，故喂假事实就能把规矩推理干净：
//!
//! ```text
//!   Who      谁在问        ← 假名册（一张 TID → 号 的表）
//!   Branch   在不在一支里   ← 假谱系（一张 子 → 父 的表）
//!   League   在不在这枚盟里 ← 假盟册（一张成员表）
//!   Control  上面三条的出口 ← gate 那一侧的粘合（本台与树域各一份实现）
//!   VestedBy 那一枚还答得出吗 ← 账那一侧"主人还在不在场"（与树收的是同一枚函数指针）
//! ```
//!
//! 四组判据、按重要性排：
//!
//! 1. **三格答案各归各位**：`Allow` / `Deny`（终态）/ `Unjudged`（判不了——因分"会好的"与
//!    "好不了的"两类，见 `judge::Ruling`）——若把"判不了"读成"你没资格"，整机就会把
//!    "身份服务挂了"报成"没权限"；
//! 2. **没身份就是没资格**：没绑过的 TID 在 `Rule::Public` 上也要被拒（编排域落的正是这一格）
//!    ——这一条松掉，门禁就成了一条"不绑身份即可绕过"的后门；
//! 3. **`Ok(false)` 与 `Err` 分家**：前者是"不在"，后者是"问不到"；再加上「手里没有门牌」
//!    那一格（[`Code::Blind`]）——它**不是放行**；
//! 4. **账是缓存，树才是真相**：那一行对不上真相时答**默认值**并**顺手销账**——这条与
//!    `find` 的惰性剔死、"主人不在场"是同一个形状（"不问就不动，问了才发现"）。
//!
//! # 为什么这一台也要编 `core.rs`
//!
//! `ledger.rs` 的两把钥匙是 [`Where`] / [`EntryId`]，失败域是 [`Fail`]——那三样住在
//! `operator/core.rs` 里。**多编一份它是挑过的**：`core.rs` 里**没有**测试（它的用例住这台
//! 靶里 `operator` 那一份），故引它**不带进重复用例**；反过来把 `judge` 那几份并进
//! `tests/operator.rs` 会把上面这些**再跑一遍**。
//!
//! # 为什么这一台不编 `principal/core.rs`
//!
//! `judge.rs` 里两个号是**泛型**（`Rule<P, C>`），本台就用 `u64` 当那两个号（= [`Id`]，
//! 也是线上那一格的宽度）。这不是省事：若判据直接写死 `PrincipalId` / `CoalitionId`，本台
//! 就得跟着编那两份核心源码，而那两份的用例住在**别的靶**里（`roster`）——把新判据挂在
//! 别处跑着的桩上不划算（照实记见 `judge.rs` 头注）。

extern crate alloc;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// 这一台要的五份——`judge`（判据的正文）/ `gate`（裁决 → 线上那一格）/ `core`（**只为账要的
/// 那三样**：`Where` / `EntryId` / `Fail` / `VestedBy`，本台不测树本身）/ `ledger`（那一本账）/
/// `frame`（帧那一半）——全是**真依赖** `contract::system::operator`（逐字同一份源码）。
///
/// **模块名照旧**（`core` / `gate` / `judge` / `ledger` / `frame`）：那几份源码里的
/// `use super::…` / `use crate::id::Id` **在 `contract` 里本来就成立**，靶这侧那些
/// `use crate::core::…` 也一个字不改。**靶自己不再需要 `id` 那一片**——`#[path]` 那一版要它，
/// 是因为源码里的 `crate::id` 落在**靶**的根上；现在落在 `contract` 的根上。
///
/// **照实记（`#[path]` 退场）**：这六份原先各拿一行 `#[path]` 逐字编进靶（外加 `fail_codes!`
/// 码表一份、`core` 上一个 `#[allow(dead_code)]`——本台只借它的类型，那一族方法在这台里
/// 全是"没被叫过"）。真依赖挂上之后这些一起退场：`dead_code` 按**定义它的 crate** 算，
/// `contract` 是依赖、不重算；那批用例也不会被带进来（`contract` 是 `test = false`）。
/// **为什么当初要编 `core.rs`**：`ledger.rs` 的两把钥匙是 `Where` / `EntryId`、失败域是 `Fail`，
/// 三样都住 `operator/core.rs`；**这就是"这一台要它"的全部理由**，与"要不要测树"无关。
///
/// **为什么这一台不编 `principal/core.rs`**：`judge.rs` 里两个号是**泛型**（`Rule<P, C>`），
/// 本台就用 `u64` 当那两个号（= `Id`，也是线上那一格的宽度）。那不是省事：若判据直接写死
/// `PrincipalId` / `CoalitionId`，这一台就得跟着编那两份核心源码，而那两份的用例住在**别的靶**
/// 里（`roster`）——把新判据挂在别处跑着的桩上不划算（照实记见 `judge.rs` 头注）。
use contract::system::operator::{core, frame, gate, judge, ledger};

use env::{Name, PieToken, TaskId};

use crate::core::{EntryId, Fail, Where};
use crate::gate::{Blind, Code, Control, WIRE_DENIED, WIRE_OK, WIRE_UNJUDGED, verdict};
use crate::judge::{Branch, Door, Id, League, Rule, Ruling, Who, judge};
use crate::ledger::{Key, Ledger};

// ── 一台"分配会失败"的台子（只给账的容量那一条用例）────────────────
//
// 账有一格判据是 **fail-closed**：要位备不下 ⇒ 这一问**就该失败**（答 `FULL`），而且
// **落树那一手一次都不该被叫**——"用"那一轴一旦漏记，那一格就从**私名**回落成**公名**
// （`Publishers` 那版是 fail-soft，照实记见 `ledger.rs::land`）。它只有把分配**真的**打掉才
// 量得到——故这里给测试靶换一个**可关掉**的全局分配器（与 `protocol-case` 的 `operator` 靶
// 那一台同一款）。
//
// 旗帜是**线程局部**的，不是进程级的：libtest 每个用例各一枚线程，而"打掉分配"若做成全局
// 旗帜，那一条用例亮旗的时候别的用例正好在分配 ⇒ 随机 panic（**随机红的门比没有门更坏**）。
// 用 `const` 初值：`thread_local!` 的惰性初始化**本身要分配**——在分配器里自指。
struct Flaky;

thread_local! {
    static NO_ROOM: Cell<bool> = const { Cell::new(false) };
}

unsafe impl GlobalAlloc for Flaky {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if NO_ROOM.with(Cell::get) {
            // 内核里那一条是 `handle_alloc_error`（abort）——宿主上等价的就是**返空**，
            // 让 `Vec::try_reserve` 如实答 `Err`。
            //
            // 照实记：这里只能写 `std::ptr`——本台的 `mod core;` 把 `core` 那个 crate 名
            // 遮住了（它编的是 `contract/src/system/operator/core.rs` 那一份）。
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        if NO_ROOM.with(Cell::get) {
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static ALLOC: Flaky = Flaky;

/// 打掉（或放开）本线程的分配。
fn no_room(on: bool) {
    NO_ROOM.with(|flag| flag.set(on));
}

// ── 假事实 ──────────────────────────────────────────────────

/// 假名册：只有这一位绑过（号 = `7`）。
struct Roster(&'static [(TaskId, Id)]);
impl Who<Id> for Roster {
    fn who(&self, tid: TaskId) -> Result<Option<Id>, ()> {
        Ok(self.0.iter().find(|(t, _)| *t == tid).map(|(_, p)| *p))
    }
}

/// 假名册：**问不到**（身份服务不在 / 超时）。
struct Broken;
impl Who<Id> for Broken {
    fn who(&self, _: TaskId) -> Result<Option<Id>, ()> {
        Err(())
    }
}

/// 假谱系：一张 子 → 父 的表（走到头 = 根）。
struct Chain(&'static [(Id, Id)]);
impl Branch<Id> for Chain {
    fn heir(&self, a: Id, b: Id) -> Result<bool, ()> {
        let mut at = Some(b);
        while let Some(cur) = at {
            if cur == a {
                return Ok(true);
            }
            at = self.0.iter().find(|(k, _)| *k == cur).map(|(_, v)| *v);
        }
        Ok(false)
    }
}

/// 假盟册：只有一枚盟（号 `3`），成员写死在表里。
struct Book(&'static [Id]);
impl League<Id, Id> for Book {
    fn amid(&self, me: Id, at: Id) -> Result<bool, ()> {
        Ok(at == C && self.0.contains(&me))
    }
}

/// 假树：一张 **格号 → 开者** 的表；表外的号答"没有那一位"（`Ok(None)`，与真树三因同落）。
struct Doors(&'static [(usize, TaskId)]);
impl Door for Doors {
    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()> {
        Ok(self.0.iter().find(|(e, _)| *e == at.get()).map(|(_, t)| *t))
    }
}

/// 假树：**问不到**（树自己出了事）。
struct Deaf;
impl Door for Deaf {
    fn opens(&self, _: EntryId) -> Result<Option<TaskId>, ()> {
        Err(())
    }
}

// ── 三个身份 / 一个不存在的 TID ─────────────────────────────

const ME: TaskId = TaskId::new(22); // 绑在号 7 上
const OTHER: TaskId = TaskId::new(33); // 绑在别的号上
const UNBOUND: TaskId = TaskId::new(44); // 从没绑过

const P: Id = 7; // ME 的号
const Q: Id = 9; // P 的父（故 Q ≼ P）
const C: Id = 3; // 那枚盟

fn go(tid: TaskId, rule: Rule<Id, Id>) -> Ruling {
    let roster = Roster(&[(ME, P), (OTHER, 11)]);
    let chain = Chain(&[(P, Q)]); // P 的父是 Q ⇒ heir(Q, P) = true

    let book = Book(&[P]);
    // 第 5 格的门牌是 **OTHER 开的** ⇒ `Opens(5)` 是"许给那一位（不是我）"。
    let doors = Doors(&[(5, OTHER)]);
    judge(tid, rule, &roster, &chain, &book, &doors)
}

// ── 判据 ────────────────────────────────────────────────────

#[test]
fn public_still_requires_a_bound_identity() {
    // **这一条是门禁的底线**：公开放的只有"已绑身份"，不是"任何人"。
    // 编排域（装配者）没绑身份——它做不了树的客人，这是有意的（不是树的客人就不该有特例）。
    assert_eq!(go(ME, Rule::Public), Ruling::Allow);
    assert_eq!(
        go(OTHER, Rule::Public),
        Ruling::Allow,
        "别的已绑身份也算公开"
    );
    assert_eq!(go(UNBOUND, Rule::Public), Ruling::Deny, "没绑 = 没资格");
}

#[test]
fn is_looks_at_the_identity_not_the_task() {
    assert_eq!(go(ME, Rule::Is(P)), Ruling::Allow);
    assert_eq!(go(OTHER, Rule::Is(P)), Ruling::Deny);
    assert_eq!(go(ME, Rule::Is(Q)), Ruling::Deny);
}

#[test]
fn under_is_reflexive_and_walks_up_the_branch() {
    // `Under(p)` 问的是"这一位在 p 那一支里"，含相等（`heir` 自反）。
    assert_eq!(go(ME, Rule::Under(P)), Ruling::Allow, "自己那一支");
    assert_eq!(go(ME, Rule::Under(Q)), Ruling::Allow, "父那一支也算");
    assert_eq!(go(ME, Rule::Under(11)), Ruling::Deny, "别的支不算");
}

#[test]
fn in_looks_at_the_league() {
    assert_eq!(go(ME, Rule::In(C)), Ruling::Allow);
    assert_eq!(
        go(ME, Rule::In(4)),
        Ruling::Deny,
        "没铸过的盟：false，不是失败"
    );
}

#[test]
fn opens_is_about_who_holds_the_door() {
    // 第 5 格是 OTHER 的门牌 ⇒ 它过；别人（有身份、但不是那一位）拒。
    // 这一格与 `Is` 的分野就在这儿：`Is` 比"一条身份号"，`Opens` 比"谁占着那一格"。
    assert_eq!(go(OTHER, Rule::Opens(EntryId::new(5))), Ruling::Allow);
    assert_eq!(go(ME, Rule::Opens(EntryId::new(5))), Ruling::Deny);
    assert_eq!(go(UNBOUND, Rule::Opens(EntryId::new(5))), Ruling::Deny);
}

#[test]
fn a_missing_door_is_unjudged_but_a_doorless_opener_is_denied() {
    // 三态各归各位：**"没有那一位"判不了**（挂门牌有先后；这一格里两种因**永远好不了**
    // ——碑 / 窗格 / 门封印）；**"那一位没身份"是终态拒**；**树问不到**也是判不了。
    let roster = Roster(&[(ME, P)]);
    let chain = Chain(&[]);
    let book = Book(&[P]);
    assert_eq!(
        judge(
            ME,
            Rule::Opens(EntryId::new(6)),
            &roster,
            &chain,
            &book,
            &Doors(&[(5, OTHER)])
        ),
        Ruling::Unjudged,
        "那一格不在 / 是块 Pane / 门封印了 —— 同一个 `Ok(None)`"
    );
    assert_eq!(
        judge(
            ME,
            Rule::Opens(EntryId::new(5)),
            &roster,
            &chain,
            &book,
            &Doors(&[(5, UNBOUND)])
        ),
        Ruling::Deny,
        "开者没绑身份 ⇒ 那位没有资格"
    );
    assert_eq!(
        judge(
            ME,
            Rule::Opens(EntryId::new(5)),
            &roster,
            &chain,
            &book,
            &Deaf
        ),
        Ruling::Unjudged,
        "树自己问不到 ⇒ 判不了"
    );
}

#[test]
fn an_unreachable_roster_is_unjudged_never_denied() {
    // **判不了 ≠ 你没资格**。这一条把"身份服务挂了/超时"与"你没有权限"分开——客人在这一侧
    // 的下一步其实是同一个（当趟放弃），分开它们的是**为什么**：码不许把前者读成后者。
    let roster = Broken;
    let chain = Chain(&[]);
    let book = Book(&[P]);
    for rule in [
        Rule::Public,
        Rule::Is(P),
        Rule::Under(P),
        Rule::In(C),
        Rule::Opens(EntryId::new(5)),
    ] {
        // `Public` 也要先问身份 ⇒ 问不到同样是"判不了"，不是"过"。
        assert_eq!(
            judge(ME, rule, &roster, &chain, &book, &Deaf),
            Ruling::Unjudged,
            "问不到身份：判不了"
        );
    }
    // **同一件事在 gate 那一层也成立**（原 `gate.rs` 的
    // `a_mute_identity_service_is_unjudged_not_denied`，用户裁定后并进这里）：四问全 `Err`
    // ⇒ 判不了，不是拒。这一格钉的是"服务挂了"与"你没资格"分家。
    assert_eq!(verdict(&Mute, ME, Rule::Public), Code::Unjudged);
}

#[test]
fn an_unbound_caller_never_reaches_the_predicates() {
    // 顺序契约的一半：没身份（`Ok(None)`）⇒ 当场拒，**两条谓词边一次都不发**。
    // 这里用一个"谓词会记数"的桩把它量出来：谓词一次都没被叫。
    let roster = Roster(&[]);
    let chain = Counting::new();
    let book = Counting2::new();
    assert_eq!(
        judge(UNBOUND, Rule::Under(P), &roster, &chain, &book, &Deaf),
        Ruling::Deny
    );
    assert_eq!(chain.calls(), 0, "没身份时不该问谱系");
    assert_eq!(book.calls(), 0, "没身份时不该问盟册");
}

/// 会记账的假谱系（只给上一条用例用）。
struct Counting(std::sync::atomic::AtomicUsize);
impl Counting {
    fn new() -> Counting {
        Counting(std::sync::atomic::AtomicUsize::new(0))
    }
    fn calls(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl Branch<Id> for Counting {
    fn heir(&self, _: Id, _: Id) -> Result<bool, ()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(false)
    }
}

/// 会记账的假盟册（只给上一条用例用）。
struct Counting2(std::sync::atomic::AtomicUsize);
impl Counting2 {
    fn new() -> Counting2 {
        Counting2(std::sync::atomic::AtomicUsize::new(0))
    }
    fn calls(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl League<Id, Id> for Counting2 {
    fn amid(&self, _: Id, _: Id) -> Result<bool, ()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(false)
    }
}

// ── 裁决 → 线上那一格（gate 那一层）────────────────────────

/// 把本台那三个假事实收成一个 [`Control`]——与树域那一侧的真实现同形（那是两枚门牌）。
struct Facts;
impl Control for Facts {
    fn who(&self, tid: TaskId) -> Result<Option<Id>, ()> {
        Ok([(ME, P), (OTHER, 11)]
            .iter()
            .find(|(t, _)| *t == tid)
            .map(|(_, p)| *p))
    }
    fn heir(&self, a: Id, b: Id) -> Result<bool, ()> {
        let mut at = Some(b);
        while let Some(cur) = at {
            if cur == a {
                return Ok(true);
            }
            at = [(P, Q)].iter().find(|(k, _)| *k == cur).map(|(_, v)| *v);
        }
        Ok(false)
    }
    fn amid(&self, me: Id, at: Id) -> Result<bool, ()> {
        Ok(me == P && at == C)
    }
    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()> {
        // 第 5 格是 OTHER 的门牌（与 `go` 那一张假树同一格）。
        Ok((at.get() == 5).then_some(OTHER))
    }
}

/// 一颗**答不上来**的身份边（服务在，但这一问没答）。
///
/// **照实记（用户裁定"测试和运行环境分开"）**：这一颗原先住 `gate.rs` 的 `#[cfg(test)]`
/// （`a_mute_identity_service_is_unjudged_not_denied`）——那一份源里从此没有测试，这一段搬来
/// 这里。它与 `Blind` 是**两件事**：`Blind` 是"装配期还没门牌"（那一格是 `Code::Blind`），
/// 这一颗是"门牌在、问了没人答"（`Err` ⇒ 判不了）。
struct Mute;
impl Control for Mute {
    fn who(&self, _: TaskId) -> Result<Option<Id>, ()> {
        Err(())
    }
    fn heir(&self, _: Id, _: Id) -> Result<bool, ()> {
        Err(())
    }
    fn amid(&self, _: Id, _: Id) -> Result<bool, ()> {
        Err(())
    }
    fn opens(&self, _: EntryId) -> Result<Option<TaskId>, ()> {
        Err(())
    }
}

#[test]
fn the_verdict_maps_onto_the_wire_cells() {
    // 放行 / 终态拒 / 判不了——三格各归各位（真值来自上面那三张假表）。
    assert_eq!(verdict(&Facts, ME, Rule::Public), Code::Ok);
    assert_eq!(verdict(&Facts, UNBOUND, Rule::Public), Code::Denied);
    assert_eq!(verdict(&Facts, ME, Rule::Under(Q)), Code::Ok);
    assert_eq!(verdict(&Facts, ME, Rule::In(C)), Code::Ok);
    assert_eq!(verdict(&Facts, ME, Rule::Under(11)), Code::Denied);
    // 第五格：那一格是 OTHER 的门牌 ⇒ 它过、ME 拒；"没有那一位"是判不了（不是拒）。
    assert_eq!(
        verdict(&Facts, OTHER, Rule::Opens(EntryId::new(5))),
        Code::Ok
    );
    assert_eq!(
        verdict(&Facts, ME, Rule::Opens(EntryId::new(5))),
        Code::Denied
    );
    assert_eq!(
        verdict(&Facts, ME, Rule::Opens(EntryId::new(6))),
        Code::Unjudged
    );

    // 三格码 ↔ **线上那一格**。**照实记（用户裁定"测试和运行环境分开"）**：这四条原先住
    // `gate.rs` 的 `#[cfg(test)]`（`the_three_codes_map_to_the_three_wire_cells`），那一份源里
    // 从此没有测试；它们与这一条问的是同一件事（裁决 → 线上那一格），故并进这里。
    assert_eq!(Code::Ok.wire(), WIRE_OK);
    assert_eq!(Code::Denied.wire(), WIRE_DENIED);
    assert_eq!(Code::Unjudged.wire(), WIRE_UNJUDGED);
    // 「手里没有门牌」在**客人那一侧**与"判不了"同一格——但树侧的读数分得开。
    assert_eq!(Code::Blind.wire(), WIRE_UNJUDGED);
}

#[test]
fn no_face_at_all_is_blind_and_never_allow() {
    // **手里还没门牌**：判不了任何人。这一格**不是放行**——松成放行就等于"协调服务没配上
    // ⇒ 门禁不存在"。**照实记**：生产路径到不了这一格——装配期由 `may` 在更早处短路
    // （装配期 principal 挂自己门牌时既没门牌又没身份，见 `operator/server.rs` 的 `may`），
    // 故 [`Blind`] 的读者是**这一台**：它钉的是"这一格在码这一层永远不是 0"。
    assert_eq!(verdict(&Blind, ME, Rule::Public), Code::Blind);
    assert!(!verdict(&Blind, ME, Rule::Public).passed());
    // **一颗盲的门牌对谁都盲**（原 `gate.rs` 的 `the_blind_control_is_blind` 里那两条，
    // 用户裁定"测试和运行环境分开"后并进这里）：不只是 `Public` 那一格。
    assert_eq!(verdict(&Blind, ME, Rule::Under(P)), Code::Blind);
    assert_eq!(
        verdict(&Blind, ME, Rule::Opens(EntryId::new(5))),
        Code::Blind
    );
}

// ── 那一本账（ledger 那一层）────────────────────────────────
//
// 这四组钉的是账的三条契约：**两种寻址打的是同一格**、**陈旧那一行答默认 + 顺手销账**、
// **"活着"那一问就是主人还在不在场**、**先要位才写得进**。
//
// 照实记（为什么这批判据值得写）：账的前身（`server.rs` 的 `Publishers`）住在适配层，
// **编不进宿主靶**——而它在真机上量错过两次（第一版按号记 ⇒ 整道判据被跳过，`probe-owner`
// 当场顶掉了 `/device/uart` 的牌子）。这是它第一次进宿主靶的判据。

/// 造一枚号给账用：**唯一的门是"收号"**（与 `operator` 靶那一台同一条路）。
fn pie(n: usize) -> PieToken {
    PieToken::from_bytes(&(n as u64).to_le_bytes()).expect("8 字节")
}

/// 造一段名字。
fn name(s: &str) -> Name {
    Name::new(s).expect("名字合法")
}

/// 假"活着"那一问：`VestedBy` 是**函数指针**（捕不了环境），故用一张静态位表。
/// 只有被 `alive()` 点亮过的那几枚答得出。
static ALIVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn alive(n: usize) {
    ALIVE.fetch_or(1 << n, std::sync::atomic::Ordering::Relaxed);
}

fn dead(n: usize) {
    ALIVE.fetch_and(!(1 << n), std::sync::atomic::Ordering::Relaxed);
}

fn vested_by(pie: PieToken) -> Option<TaskId> {
    (ALIVE.load(std::sync::atomic::Ordering::Relaxed) & (1 << pie.get()) != 0).then_some(ME)
}

/// 那一格还在树上吗——账对真相的那一问（本台用它当 `fresh`）。
/// 每一项 = "这一号还是原来那一格"。
struct Truth(std::sync::Mutex<Vec<EntryId>>);
impl Truth {
    fn new(ids: &[EntryId]) -> Truth {
        Truth(std::sync::Mutex::new(ids.to_vec()))
    }
    /// 那一格从树上消失（`trim` / 惰性剔死 / 被 `part` 顶掉）。
    fn gone(&self, id: EntryId) {
        self.0.lock().unwrap().retain(|x| *x != id);
    }
    fn fresh(&self, id: EntryId) -> bool {
        self.0.lock().unwrap().contains(&id)
    }
}

const AT: Where = Where::At(EntryId::new(4)); // 那一块 Pane（号 4）
const ID: EntryId = EntryId::new(9); // 那一格自己的号

/// 一本只记了一行的账：`(坐标, 名)` 与 `号` 指着同一条。
///
/// **落树那一手是注入的**（[`Ledger::land`] 收的那个闭包）：本台**不测树本身**，故这里直接交出
/// 那一格自己的号——而要位与记账两步走的是**真**道路，不是替身。
fn one_line(rule: Rule<Id, Id>, mine: bool, n: usize) -> Ledger<Id, Id> {
    let mut book: Ledger<Id, Id> = Ledger::new(vested_by);
    let id = book
        .land(AT, name("uart"), pie(n), rule, mine, ME, || Ok(ID))
        .expect("腾得出一行");
    assert_eq!(id, ID);
    book
}

#[test]
fn an_empty_ledger_answers_public_and_free() {
    // **两条"没这一格"都不是失败**：`part` 出来的格子本来就没有规矩；占了它的人也没声明归属。
    let mut book: Ledger<Id, Id> = Ledger::new(vested_by);
    let truth = Truth::new(&[ID]);
    assert_eq!(book.rule(Key::Id(ID), |id| truth.fresh(id)), Rule::Public);
    assert_eq!(
        book.rule(Key::At(AT, name("uart")), |id| truth.fresh(id)),
        Rule::Public
    );
    assert!(book.claimable(Key::Id(ID), OTHER, |id| truth.fresh(id)));
}

#[test]
fn both_keys_reach_the_same_line() {
    // **这一条是这一格存在的全部理由**：`land` 手里只有坐标，`find` / `trim` 手里只有号
    // ——两种寻址打的是同一格。第一版按号记，`land` 那一问因此整道判据被跳过（真机读数
    // `probe-owner: tree land=OK id=5`）。
    for rule in [
        Rule::Public,
        Rule::Is(P),
        Rule::Under(Q),
        Rule::In(C),
        Rule::Opens(EntryId::new(5)),
    ] {
        let mut book = one_line(rule, true, 1);
        assert_eq!(book.rule(Key::Id(ID), |_| true), rule, "按号查");
        assert_eq!(
            book.rule(Key::At(AT, name("uart")), |_| true),
            rule,
            "按坐标查：同一条"
        );
        // 两把钥匙也答同一个"归谁"。
        assert!(book.claimable(Key::Id(ID), ME, |_| true));
        assert!(book.claimable(Key::At(AT, name("uart")), ME, |_| true));
    }
}

#[test]
fn a_stale_line_answers_the_default_and_is_swept() {
    // **账是缓存，树才是真相**：树说那一格已经不在了 ⇒ 答默认值，并顺手把那一行销掉。
    // 与 `find` 的惰性剔死、"主人不在场"同一条形状（不问就不动，问了才发现）。
    let truth = Truth::new(&[ID]);
    let mut book = one_line(Rule::Is(P), true, 1);
    assert_eq!(book.len(), 1, "先记了一条");
    assert_eq!(
        book.rule(Key::At(AT, name("uart")), |id| truth.fresh(id)),
        Rule::Is(P),
        "对得上真相 ⇒ 照它答"
    );

    truth.gone(ID); // 树上那一格没了（`trim` / 惰性剔死 / 被 `part` 顶掉）
    assert_eq!(
        book.rule(Key::At(AT, name("uart")), |id| truth.fresh(id)),
        Rule::Public,
        "陈旧 ⇒ 默认，而不是照旧答那条规矩"
    );
    assert_eq!(book.len(), 0, "顺手销账");

    // 同一条：陈旧的归属也不该拦人（否则 `trim` 掉自己的格子之后那一格永远落不进来）。
    let mut book = one_line(Rule::Public, true, 1);
    assert!(book.claimable(Key::At(AT, name("uart")), OTHER, |id| truth.fresh(id)));
    assert_eq!(book.len(), 0);
}

#[test]
fn a_living_owner_holds_the_slot_and_a_dead_one_does_not() {
    // "规矩属于**活着的**主人"：主人答得出 ⇒ 别人接手被拒；答不出 ⇒ 谁都能接手。
    // （退场钩子会把那一域开的资源封印，故这一问一次就有答案，不要看门狗。）
    let truth = Truth::new(&[ID]);
    alive(1);
    let mut book = one_line(Rule::Public, true, 1);
    assert!(
        book.claimable(Key::At(AT, name("uart")), ME, |id| truth.fresh(id)),
        "主人本人"
    );
    assert!(
        !book.claimable(Key::At(AT, name("uart")), OTHER, |id| truth.fresh(id)),
        "主人还在场 ⇒ 别人顶不掉"
    );

    dead(1);
    let mut book = one_line(Rule::Public, true, 1);
    assert!(
        book.claimable(Key::At(AT, name("uart")), OTHER, |id| truth.fresh(id)),
        "主人不在场 ⇒ 可接手"
    );
}

#[test]
fn a_rebind_rewrites_the_same_line_and_can_give_up_the_slot() {
    // 重绑 = 改写同一条（不是再记一条）；`mine = false` 是**放弃归属**——从此谁都能落。
    let truth = Truth::new(&[ID]);
    alive(2);
    let mut book = one_line(Rule::Public, true, 2);
    // 重绑走的是**同一条**动词：同一个 (坐标, 名) ⇒ `write` 覆写那一行，不是再记一条。
    book.land(AT, name("uart"), pie(2), Rule::Is(P), false, ME, || Ok(ID))
        .expect("重绑腾得出一行");
    assert_eq!(book.len(), 1, "重绑不该多出一条");
    assert_eq!(
        book.rule(Key::At(AT, name("uart")), |id| truth.fresh(id)),
        Rule::Is(P)
    );
    assert!(
        book.claimable(Key::At(AT, name("uart")), OTHER, |id| truth.fresh(id)),
        "放弃之后别人落得进来"
    );
}

#[test]
fn dropping_a_line_makes_the_slot_free_again() {
    // 格子从树上消失的任一条路都该顺手叫一句 `drop`（`trim` / `find` 答 `Dead` / `part` 顶掉）。
    let truth = Truth::new(&[ID]);
    alive(3);
    let mut book = one_line(Rule::Is(P), true, 3);
    book.drop(ID);
    assert_eq!(book.len(), 0);
    assert!(book.claimable(Key::Id(ID), OTHER, |id| truth.fresh(id)));
    assert_eq!(book.rule(Key::Id(ID), |id| truth.fresh(id)), Rule::Public);
}

#[test]
fn a_ledger_that_cannot_reserve_answers_full_and_never_reaches_the_tree() {
    // **这一格是"用"那一轴唯一的护栏**：要位失败 ⇒ 这一问**就该失败**（答 `FULL`），不能悄悄
    // 放过——放过的后果是"树上有、账上没有"，也就是**私名变公名**。
    //
    // **落树那一步现在量得到**：它是注入的一手，"它一次都没被叫"是一个可读的读数——从前那个
    // `Blank` 令牌量不到这个（它只管"没要位就写"编不过，不管"落树先于要位"）。
    let mut book: Ledger<Id, Id> = Ledger::new(vested_by);
    let mut planted = false;
    no_room(true);
    assert_eq!(
        book.land(AT, name("uart"), pie(1), Rule::Is(P), true, ME, || {
            planted = true;
            Ok(ID)
        })
        .err(),
        Some(Fail::Full),
        "备不下 ⇒ 如实报"
    );
    no_room(false);
    assert!(
        !planted,
        "要位失败 ⇒ 落树那一手一次都没被叫（树一个字没动）"
    );
    assert_eq!(book.len(), 0, "失败不留半个状态");

    // 而且**只失败这一次**：放开之后照样落得进（要位失败不是锁死）。
    assert_eq!(
        book.land(AT, name("uart"), pie(1), Rule::Is(P), true, ME, || Ok(ID)),
        Ok(ID),
        "放开之后腾得出一行"
    );
    assert_eq!(book.len(), 1);
    assert_eq!(book.rule(Key::Id(ID), |_| true), Rule::Is(P));
}

// ── 帧那一半（`operator/frame.rs`）──────────────────────────
//
// **照实记（这一组为什么值当）**：机器那几道门走的是**顺路**——客侧编一帧、持树者解一帧，
// 形状对了就继续。下面这些格子机器**一条都走不到**：短帧 / 长帧 / 动作码不对、**路数超上限
// 而那一格仍报真实段数**（那一格是 `FULL` 的判据）、**规矩那一格认不出的标号退回 `Public`**、
// 列答的"条数与帧长对不上就是读不懂"、以及失败码表的两端。

use crate::frame as f;
use contract::message::Message;

fn road(names: &[&str]) -> Vec<Name> {
    names
        .iter()
        .map(|n| Name::new(n).expect("名字合法"))
        .collect()
}

/// 编一问（**一族一只缓冲**：`Req::Buf` 就是最长那一枚）。
fn store(m: f::Req<'_>) -> ([u8; f::REQ_LEN], usize) {
    let mut buf = [0u8; f::REQ_LEN];
    let n = m.store(&mut buf).expect("这一族的缓冲就是最长那一枚");
    (buf, n)
}

/// 解一问。
fn fetch(bytes: &[u8]) -> Option<f::Wire> {
    <f::Req<'_> as Message>::fetch(bytes)
}

#[test]
fn every_ask_shape_round_trips() {
    // seek：路 + 真实段数。
    let (buf, len) = store(f::Req::Road(&road(&["sys", "operator"])));
    assert_eq!(len, 2 + 2 * env::wire::NAME_LEN);
    assert_eq!(
        fetch(&buf[..len]),
        Some(f::Wire::Road(
            {
                let mut out = [Name::EMPTY; crate::core::Operator::ROAD_MAX];
                out[0] = Name::new("sys").unwrap();
                out[1] = Name::new("operator").unwrap();
                out
            },
            2
        )),
        "路回来是同一串，段数也对"
    );
    // 长度为该形状该有的长度是帧的契约：长一字节也不是它。
    assert_eq!(fetch(&[&buf[..len], &[0u8]].concat()), None, "多一字节");

    // land：坐标 + 名 + 入口 + 两轴条件。
    let at = Where::At(EntryId::new(3));
    let seed = PieToken::from_bytes(&9u64.to_le_bytes()).unwrap();
    let (buf, len) = store(f::Req::Land {
        at,
        name: Name::new("uart").unwrap(),
        entry: seed,
        rule: Rule::Opens(EntryId::new(7)),
        mine: true,
    });
    assert_eq!(len, f::Land::LEN, "长度就是那张表求和出来的");
    assert_eq!(
        fetch(&buf[..len]),
        Some(f::Wire::Land {
            at,
            name: Name::new("uart").unwrap(),
            entry: seed,
            rule: Rule::Opens(EntryId::new(7)),
            mine: true,
        })
    );

    // list：只有坐标。
    let (buf, len) = store(f::Req::List(Where::Root));
    assert_eq!(len, f::List::LEN);
    assert_eq!(fetch(&buf[..len]), Some(f::Wire::List(Where::Root)));

    // find / trim / name 三者同形（解出来仍是三格；**共用一张表**）。
    let id = EntryId::new(11);
    for (ask, want) in [
        (f::Req::Find(id), f::Wire::Find(id)),
        (f::Req::Trim(id), f::Wire::Trim(id)),
        (f::Req::Name(id), f::Wire::Name(id)),
    ] {
        let (buf, len) = store(ask);
        assert_eq!(len, f::Entry::LEN, "三条共用一张表，长度只有一处");
        assert_eq!(fetch(&buf[..len]), Some(want));
    }
}

#[test]
fn a_road_longer_than_the_cap_still_reports_the_real_count() {
    // **那一格报的是真实段数**（可能超过上限）：持树者按它答 `FULL`——把帧里的段数裁成上限，
    // 超长的那一问就会被当成"正好"而悄悄按短的走。
    let cap = crate::core::Operator::ROAD_MAX;
    let long: Vec<String> = (0..cap + 3).map(|i| alloc::format!("s{i}")).collect();
    let names: Vec<&str> = long.iter().map(|s| s.as_str()).collect();
    let (buf, len) = store(f::Req::Road(&road(&names)));
    let head = f::RoadHead::fetch(&buf).expect("头两格");
    assert_eq!(head.count as usize, cap + 3, "段数那一格报真实值");
    assert_eq!(len, 2 + cap * env::wire::NAME_LEN, "荷载只装得下上限那么多");
    match fetch(&buf[..len]) {
        Some(f::Wire::Road(_, n)) => assert_eq!(n, cap + 3, "读的人看得到它超了"),
        other => panic!("该是 Road：{other:?}"),
    }
}

#[test]
fn a_frame_that_is_not_that_shape_is_not_guessed_at() {
    let (buf, len) = store(f::Req::Find(EntryId::new(1)));
    assert_eq!(fetch(&buf[..len - 1]), None, "短一字节");
    assert_eq!(fetch(&[&buf[..len], &[0u8]].concat()), None, "长一字节");
    assert_eq!(fetch(&[]), None, "空帧（连动作码都没有）");

    // **表外的动作码**：树这一族**不另立**"表外的码"那一格（对它的答话与"读不懂"同一句，
    // 见 `frame::Wire` 的照实记）——这与板那一族的 `Wire::Unknown` 是两条族规。
    let mut outside = buf;
    outside[0] = 200;
    assert_eq!(fetch(&outside[..len]), None, "没见过的动作码 ⇒ 读不懂");

    // **坐标那一格的记**：`0` / `1` 之外不是任何一种坐标 ⇒ 整帧读不懂（`Where` 的 `Field`）。
    let (list, len) = store(f::Req::List(Where::Root));
    let mut bad_at = list;
    bad_at[1] = 9;
    assert_eq!(fetch(&bad_at[..len]), None, "记既不是根也不是号");

    // **名那一格读不成一个 `Name`** ⇒ 整帧读不懂（空名 / 串尾有垃圾 / 不是 UTF-8 在这一格归一）。
    let (part, len) = store(f::Req::Part {
        at: Where::Root,
        name: Name::new("x").unwrap(),
    });
    let mut unnamed = part;
    unnamed[10..42].fill(0);
    assert_eq!(fetch(&unnamed[..len]), None, "那一格不是名字");

    // **少一字节就不是那一帧**（长度是各张表的契约）：`land` 那一帧短一格也读不懂。
    let (land, len) = store(f::Req::Land {
        at: Where::Root,
        name: Name::new("x").unwrap(),
        entry: PieToken::from_bytes(&1u64.to_le_bytes()).unwrap(),
        rule: Rule::Public,
        mine: false,
    });
    assert_eq!(fetch(&land[..len - 1]), None, "短一字节的 land");

    // **照实记（这一格原先的说法是假的）**：旧注写着"入口那一枚必须带（全 0 ⇒ 解不出令牌）"，
    // 而全零解出来的是一枚**合法的**令牌（0 = "没有"那一格）——真正挡住"缺入口"的是**长度**
    // （那一条帧根本到不了 60 字节），不是值。判"这枚号合不合法"是核心那一侧的事（见
    // `system::board::frame` 同款的一句：那一格由核心答 `Denied`）。
    let mut zero_entry = land;
    zero_entry[42..50].fill(0);
    assert!(
        matches!(fetch(&zero_entry[..len]), Some(f::Wire::Land { .. })),
        "全零的入口解得出来 ⇒ 它是不是一枚可用的号由核心答"
    );
}

#[test]
fn the_coord_frame_carries_one_number_and_one_pair_of_eyes() {
    use plan::assembly::Eyes;

    // 长度是**字段宽度之和**（一处定义）——不是这里写的 16。
    assert_eq!(f::CoordFrame::LEN, 16, "一位域（8）＋ 一双眼睛（8）");

    for eyes in [Eyes::Roster, Eyes::League] {
        let mut buf = [0u8; f::CoordFrame::LEN];
        f::CoordFrame {
            who: TaskId::new(7),
            eyes,
        }
        .store(&mut buf);
        let back = f::CoordFrame::fetch(&buf).expect("写进去的读得回来");
        assert_eq!(back.who, TaskId::new(7), "那一位域自己的号");
        assert_eq!(back.eyes, eyes, "哪一双眼睛（两枚共读同一处定义）");
    }

    // **表外的眼睛码 ⇒ 整帧读不懂**，不猜成某一枚：持树者那一侧据此报一句、不静默
    // （`programs/src/system/operator/server.rs` 的 `settle`）。这一格是本刀唯一新长出来的
    // 可读错的形状——`of_wire` 若写成"非 0 即盟册"，这里当场红。
    let mut outside = [0u8; f::CoordFrame::LEN];
    outside[8] = 2;
    assert_eq!(f::CoordFrame::fetch(&outside), None, "表外的眼睛码");

    // 缺一字节就是缺一字节：偏移由 `Field::WIDTH` 求和，故"短"只能是读不懂。
    assert_eq!(
        f::CoordFrame::fetch(&outside[..f::CoordFrame::LEN - 1]),
        None,
        "短一字节"
    );
    assert_eq!(f::CoordFrame::fetch(&[]), None, "空帧");
}

#[test]
fn the_rule_cell_round_trips_and_an_unknown_tag_falls_back_to_public() {
    // 五格都来回一趟（`Opens` 那一格进的是**门的号**）。
    for rule in [
        Rule::Public,
        Rule::Is(7),
        Rule::Under(8),
        Rule::In(9),
        Rule::Opens(EntryId::new(10)),
    ] {
        let (buf, len) = store(f::Req::Land {
            at: Where::Root,
            name: Name::new("x").unwrap(),
            entry: PieToken::from_bytes(&1u64.to_le_bytes()).unwrap(),
            rule,
            mine: false,
        });
        match fetch(&buf[..len]) {
            Some(f::Wire::Land { rule: back, .. }) => assert_eq!(back, rule, "{rule:?} 来回一趟"),
            other => panic!("该是 Land：{other:?}"),
        }
    }

    // 「用」那一轴在整帧里的起点按**文件头那张布局图**算：`[10 .. 42]` 名、`[42 .. 50]` 入口、
    // `[50]` 改、`[51 .. 60]` 用——顺带把"布局与文档一致"也钉住。
    let rule_at = 10 + env::wire::NAME_LEN + 8 + 1;
    let (mut buf, len) = store(f::Req::Land {
        at: Where::Root,
        name: Name::new("x").unwrap(),
        entry: PieToken::from_bytes(&1u64.to_le_bytes()).unwrap(),
        rule: Rule::Public,
        mine: false,
    });
    assert_eq!(len, f::Land::LEN);

    // **认不出的标号退回 `Public`**（不是 `None`：这一格是"没有条件"，不是"读不懂"）。
    buf[rule_at] = 99;
    match fetch(&buf[..len]) {
        Some(f::Wire::Land { rule, .. }) => assert_eq!(rule, Rule::Public),
        other => panic!("该是 Land：{other:?}"),
    }

    // **老帧那一格退了**（照实记）：从前 50/51 字节起就收——最后那两轴读不到就按 `Public` 走
    // （那是给"还没写这两轴的调用方"留的兜底）。字段表把长度变成**契约**：短一字节整帧读不懂，
    // 故 51 字节的老帧不再解得出。**这是这一刀收紧的一格**，不是漏了。
    assert_eq!(fetch(&buf[..rule_at]), None, "51 字节的老帧");
}

/// 编一答（**一族一只缓冲**：`Rep::Buf` 就是最大那一形）。
fn say(rep: f::Rep) -> ([u8; f::REP_LEN], usize) {
    let mut buf = [0u8; f::REP_LEN];
    let n = rep.store(&mut buf).expect("这一族的缓冲就是最大那一形");
    (buf, n)
}

/// 收一答（**原样的字节**——形状由问的人按自己的读法认）。
fn heard(bytes: &[u8]) -> f::Said {
    <f::Rep as Message>::fetch(bytes).expect("这一族的缓冲就是最大那一形")
}

#[test]
fn the_list_answer_refuses_a_count_that_disagrees_with_the_frame() {
    let ids = [EntryId::new(2), EntryId::new(4), EntryId::new(6)];
    let (buf, n) = say(f::Rep::List(f::Listing::of(ids.iter().copied())));
    assert_eq!(n, 2 + 3 * 8, "状态 ＋ 条数 ＋ 三枚号");
    let back = heard(&buf[..n]).list().expect("读得回来");
    // 几枚 = 走一遍数出来（`Listing` 的读面只留 `iter` 这一格，见它的照实记）。
    assert_eq!(back.iter().count(), 3);
    assert_eq!(back.iter().collect::<Vec<_>>(), ids.to_vec());

    assert_eq!(heard(&buf[..n - 1]).list(), Err(f::BAD), "短一字节");
    let mut liar = buf;
    liar[1] = 4; // 说有四枚，可帧里只有三枚
    assert_eq!(heard(&liar[..n]).list(), Err(f::BAD), "条数说谎");
    // 状态那一格不是 `OK` ⇒ 原样把那一格报回去（读的人按它分流）——**一格状态也是那一形**
    // （`[status]` 短于一格条数，正是 [`f::Tally`] 判它读不懂的那一条）。
    let mut failed = buf;
    failed[0] = f::FULL;
    assert_eq!(heard(&failed[..n]).list(), Err(f::FULL));
    assert_eq!(heard(&[f::FULL]).list(), Err(f::FULL), "一格状态");
}

#[test]
fn the_name_and_id_answers_are_fixed_shapes() {
    // **名那一形：长度即名长**（不是一个定长格 + 长度格）——故"多出来的"字节会被读成名字的一部分。
    let (buf, n) = say(f::Rep::Name(Name::new("uart").unwrap()));
    assert_eq!(n, 1 + 4);
    assert_eq!(heard(&buf[..n]).name(), Ok(Name::new("uart").unwrap()));

    // 多一个**零**：名字里夹 NUL ⇒ 判废；多一个**别的字节**：那就是另一个名字（读得出来）。
    assert_eq!(heard(&buf[..n + 1]).name(), Err(f::BAD), "夹了 NUL");
    let mut longer = buf;
    longer[n] = b'X';
    assert_eq!(
        heard(&longer[..n + 1]).name(),
        Ok(Name::new("uartX").unwrap())
    );

    // **号那一形是定长格**（`[status][8 字节]`）：短一字节、长一字节都是读不懂。
    // 照实记：`长一字节`这一句是牙口量出来的——把 `!= 8` 放宽成 `< 8` 之后，先前只测"短一字节"
    // 的那一版**全门照绿**。
    let (buf, n) = say(f::Rep::Entry(EntryId::new(12)));
    assert_eq!(n, f::Word::LEN);
    assert_eq!(heard(&buf[..n]).entry(), Ok(EntryId::new(12)));
    assert_eq!(heard(&buf[..n - 1]).entry(), Err(f::BAD), "短一字节");
    assert_eq!(heard(&buf[..n + 1]).entry(), Err(f::BAD), "长一字节");
    // 状态那一格不是 `OK` ⇒ 原样报回（`find` 那一档读的正是它，见 `client::find`）。
    let mut failed = buf;
    failed[0] = f::DEAD;
    assert_eq!(heard(&failed[..n]).entry(), Err(f::DEAD));
    assert_eq!(heard(&failed[..n]).code(), f::DEAD);

    // **门闩那一形**：与号那一形**逐字同形**、另一个读法（`find` 的下场）。
    let seed = PieToken::from_bytes(&9u64.to_le_bytes()).unwrap();
    let (buf, n) = say(f::Rep::Seed(seed));
    assert_eq!(n, f::Word::LEN);
    assert_eq!(heard(&buf[..n]).seed(), Ok(seed));
    assert_eq!(
        heard(&buf[..n]).entry(),
        Ok(EntryId::new(9)),
        "同一格 8 字节：是哪一种号由问的人认"
    );
}

#[test]
fn the_operator_failure_table_is_bijective_and_keeps_bad_outside() {
    use f::{
        BAD, DEAD, DENIED, FULL, NONEMPTY, NOTAPANE, NOTATILE, OK, UNKNOWN, code_to_fail,
        fail_to_code,
    };
    assert_eq!(fail_to_code(None), OK);
    for (fail, code) in [
        (Fail::Unknown, UNKNOWN),
        (Fail::NonEmpty, NONEMPTY),
        (Fail::NotATile, NOTATILE),
        (Fail::NotAPane, NOTAPANE),
        (Fail::Full, FULL),
        (Fail::Dead, DEAD),
    ] {
        assert_eq!(fail_to_code(Some(fail)), code, "{fail:?}");
        assert_eq!(code_to_fail(code), Some(fail), "一端一格");
        assert_ne!(code, OK, "失败不许与 OK 同码");
    }
    assert_eq!(code_to_fail(OK), None);
    assert_eq!(code_to_fail(BAD), None, "读不懂那一格在失败域之外");
    assert_eq!(code_to_fail(150), None, "表外的码");
}

// ── 面不相撞那一条用例搬去了**编译期**（用户裁定"常量交给编译器"）────────────
//
// `the_operator_marks_do_not_collide_with_the_other_doors` 原先在这里：它比的**全是常量**
// （`ASK_MARK` / `TIP_MARK` / `LINK` / `TIP_NAME`）。那几条现在写在
// `crates/contract/src/system/operator/frame.rs` 的 `const _: () = assert!(…)` 里——**编译期**，
// riscv 那一档也一样钉着；比它强，且不再占一条用例。
