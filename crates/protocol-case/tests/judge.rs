//! 门外那一问的门（**宿主台**）—— 裁决那三格规矩 + 它翻成线上那一格 + **那一本账**，
//! 在宿主上真跑一遍。
//!
//! # 这批判据钉的是什么
//!
//! `crates/protocol/src/system/operator/judge.rs` 是"这一位许不许动这一格"的**定义**；
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

/// 判据的正文（就是 `crates/protocol/src/system/operator/judge.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/system/operator/judge.rs"]
mod judge;

/// 裁决 → 线上那一格（就是 `crates/protocol/src/system/operator/gate.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/system/operator/gate.rs"]
mod gate;

/// 树的正文（就是 `crates/protocol/src/system/operator/core.rs` 那一份，逐字未改）——**只为账要的
/// 那三样**（`Where` / `EntryId` / `Fail` / `VestedBy`），本台不测树本身。
// 照实记：`core.rs` 那一份**只借它的类型**（本台不测树本身）⇒ 它那一族方法在这一台里
// 全是"没被叫过"。挂 `allow(dead_code)` 而不是把那几条删掉：那是**逐字未改的源码**，
// 改它就是改判据的对象。同一个文件在 `protocol-case` 的 `operator` 靶那一台里是被叫全的。
#[allow(dead_code)]
#[path = "../../protocol/src/system/operator/core.rs"]
mod core;

/// 那一本账（就是 `crates/protocol/src/system/operator/ledger.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/system/operator/ledger.rs"]
mod ledger;

/// 码表宏（`fail_codes!`）自己一份源——**协议与宿主靶同读这一份**（见那份文件的照实记）。
#[macro_use]
#[path = "../../protocol/src/fail_codes.rs"]
mod fail_codes;

/// 号的词汇（`crates/protocol/src/id.rs`，逐字未改）——`core.rs` 与 `frame.rs` 都写着
/// `use crate::id::Id`（本台是**摊平**的模块树 ⇒ `crate::id` 就是这一格）。
#[path = "../../protocol/src/id.rs"]
mod id;

/// **帧那一半**（`crates/protocol/src/system/operator/frame.rs`，逐字未改）—— 在本台里跑判据。
///
/// 这一台**本来就带着帧要的全部依赖**（`core` / `judge` / `gate` / `ledger` 都在同一层），
/// 故它是最省的一处落点；那一份写的 `use super::core::…` / `use super::judge::…` 逐字成立。
#[allow(dead_code)]
#[path = "../../protocol/src/system/operator/frame.rs"]
mod frame;

use env::{Name, PieToken, TaskId};

use crate::core::{EntryId, Fail, Where};
use crate::gate::{Blind, Code, Control, WIRE_DENIED, WIRE_OK, WIRE_UNJUDGED, verdict};
use crate::judge::{Branch, Door, Id, League, Rule, Ruling, Who, judge};
use crate::ledger::{Key, Ledger, Line};

// ── 一台"分配会失败"的台子（只给账的容量那一条用例）────────────────
//
// 账有一格判据是 **fail-closed**：`grow` 备不下 ⇒ 这一问**就该失败**（答 `FULL`），因为
// "用"那一轴一旦漏记，那一格就从**私名**回落成**公名**（`Publishers` 那版是 fail-soft，
// 照实记见 `ledger.rs::grow`）。它只有把分配**真的**打掉才量得到——故这里给测试靶换一个
// **可关掉**的全局分配器（与 `protocol-case` 的 `operator` 靶那一台同一款）。
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
            // 遮住了（它编的是 `protocol/src/system/operator/core.rs` 那一份）。
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
// 当场顶掉了 `/device/uart` 的牌子）。这是它第一次被逐字编进测试靶。

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
fn one_line(rule: Rule<Id, Id>, mine: bool, n: usize) -> Ledger<Id, Id> {
    let mut book: Ledger<Id, Id> = Ledger::new(vested_by);
    let blank = book.grow().expect("腾得出一行");
    book.write(
        blank,
        Line::new(AT, name("uart"), ID, rule, mine, ME, pie(n)),
    );
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
    let blank = book.grow().expect("腾得出一行");
    book.write(
        blank,
        Line::new(AT, name("uart"), ID, Rule::Is(P), false, ME, pie(2)),
    );
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
fn a_ledger_that_cannot_grow_answers_full_and_leaves_nothing_behind() {
    // **这一格是"用"那一轴唯一的护栏**：记账失败 ⇒ 这一问**就该失败**（答 `FULL`），
    // 不能悄悄放过——放过的后果是"树上有、账上没有"，也就是**私名变公名**。
    // 撤掉 `grow` 那一行 `try_reserve`，这条用例当场变红（分配器按旗帜返空）。
    let mut book: Ledger<Id, Id> = Ledger::new(vested_by);
    no_room(true);
    assert_eq!(book.grow().err(), Some(Fail::Full), "备不下 ⇒ 如实报");
    no_room(false);
    assert_eq!(book.len(), 0, "失败不留半个状态");

    // 而且**只失败这一次**：放开之后照样写得进（`Blank` 挡的是"没要位就写"，不是"失败即锁死"）。
    let blank = book.grow().expect("放开之后腾得出一行");
    book.write(
        blank,
        Line::new(AT, name("uart"), ID, Rule::Is(P), true, ME, pie(1)),
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

fn road(names: &[&str]) -> Vec<Name> {
    names
        .iter()
        .map(|n| Name::new(n).expect("名字合法"))
        .collect()
}

#[test]
fn every_ask_shape_round_trips() {
    // seek：路 + 真实段数。
    let (buf, len) = f::pack_ask(f::Ask::Road(&road(&["sys", "operator"])));
    assert_eq!(len, 2 + 2 * env::wire::NAME_LEN);
    assert_eq!(f::op_of(&buf[..len]), Some(f::SEEK));
    assert_eq!(
        f::unpack_ask(f::SEEK, &buf[..len]),
        Some(f::AskIn::Road(
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

    // land：坐标 + 名 + 入口 + 两轴条件。
    let at = Where::At(EntryId::new(3));
    let seed = PieToken::from_bytes(&9u64.to_le_bytes()).unwrap();
    let (buf, len) = f::pack_ask(f::Ask::Land {
        at,
        name: Name::new("uart").unwrap(),
        entry: seed,
        rule: Rule::Opens(EntryId::new(7)),
        mine: true,
    });
    assert_eq!(len, f::LAND_FRAME);
    assert_eq!(
        f::unpack_ask(f::LAND, &buf[..len]),
        Some(f::AskIn::Land {
            at,
            name: Name::new("uart").unwrap(),
            entry: seed,
            rule: Rule::Opens(EntryId::new(7)),
            mine: true,
        })
    );

    // list：只有坐标。
    let (buf, len) = f::pack_ask(f::Ask::List(Where::Root));
    assert_eq!(len, 10);
    assert_eq!(
        f::unpack_ask(f::LIST, &buf[..len]),
        Some(f::AskIn::List(Where::Root))
    );

    // find / trim / name 三者同形（解出来仍是三格）。
    let id = EntryId::new(11);
    for (ask, op, want) in [
        (f::Ask::Find(id), f::FIND, f::AskIn::Find(id)),
        (f::Ask::Trim(id), f::TRIM, f::AskIn::Trim(id)),
        (f::Ask::Name(id), f::NAME, f::AskIn::Name(id)),
    ] {
        let (buf, len) = f::pack_ask(ask);
        assert_eq!(len, 9);
        assert_eq!(f::unpack_ask(op, &buf[..len]), Some(want));
    }
}

#[test]
fn a_road_longer_than_the_cap_still_reports_the_real_count() {
    // **那一格报的是真实段数**（可能超过上限）：持树者按它答 `FULL`——把帧里的段数裁成上限，
    // 超长的那一问就会被当成"正好"而悄悄按短的走。
    let cap = crate::core::Operator::ROAD_MAX;
    let long: Vec<String> = (0..cap + 3).map(|i| alloc::format!("s{i}")).collect();
    let names: Vec<&str> = long.iter().map(|s| s.as_str()).collect();
    let (buf, len) = f::pack_ask(f::Ask::Road(&road(&names)));
    assert_eq!(buf[1] as usize, cap + 3, "段数那一格报真实值");
    assert_eq!(len, 2 + cap * env::wire::NAME_LEN, "荷载只装得下上限那么多");
    match f::unpack_ask(f::SEEK, &buf[..len]) {
        Some(f::AskIn::Road(_, n)) => assert_eq!(n, cap + 3, "读的人看得到它超了"),
        other => panic!("该是 Road：{other:?}"),
    }
}

#[test]
fn a_frame_that_is_not_that_shape_is_not_guessed_at() {
    let (buf, len) = f::pack_ask(f::Ask::Find(EntryId::new(1)));
    assert_eq!(f::unpack_ask(f::FIND, &buf[..len - 1]), None, "短一字节");
    assert_eq!(f::unpack_ask(f::FIND, &[]), None, "空帧");
    assert_eq!(f::op_of(&[]), None, "空帧连动作码都没有");

    // **动作码是调用方给的**（`unpack_ask(op, …)`）：读的人先 `op_of` 那一格，再照它分派——
    // 故解的时候不再回头看帧里那一格（照实记：我一开始把它写成"帧里那一格与调用方说的不一样
    // 就答 `None`"，实测当场红——契约不是那样）。没见过的动作码才是"读不懂"。
    let mut zero_op = buf;
    zero_op[0] = 0;
    assert_eq!(
        f::unpack_ask(f::FIND, &zero_op[..len]),
        Some(f::AskIn::Find(EntryId::new(1))),
        "解的是荷载，动作码由调用方说了算"
    );
    assert_eq!(
        f::unpack_ask(200, &buf[..len]),
        None,
        "没见过的动作码 ⇒ 读不懂"
    );
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
        let (buf, len) = f::pack_ask(f::Ask::Land {
            at: Where::Root,
            name: Name::new("x").unwrap(),
            entry: PieToken::from_bytes(&1u64.to_le_bytes()).unwrap(),
            rule,
            mine: false,
        });
        match f::unpack_ask(f::LAND, &buf[..len]) {
            Some(f::AskIn::Land { rule: back, .. }) => assert_eq!(back, rule, "{rule:?} 来回一趟"),
            other => panic!("该是 Land：{other:?}"),
        }
    }

    // 尾格的起点按**文件头那张布局图**算（`[10 .. 42]` 名 ⇒ 尾格从 `10 + NAME_LEN` 起），
    // 不引那一份内部的私有常量——顺带把"布局与文档一致"也钉住。
    let tail_at = 10 + env::wire::NAME_LEN;
    let (mut buf, len) = f::pack_ask(f::Ask::Land {
        at: Where::Root,
        name: Name::new("x").unwrap(),
        entry: PieToken::from_bytes(&1u64.to_le_bytes()).unwrap(),
        rule: Rule::Public,
        mine: false,
    });
    assert_eq!(len, f::LAND_FRAME);

    // **认不出的标号退回 `Public`**（不是 `None`：这一格是"没有条件"，不是"读不懂"）。
    buf[tail_at + 9] = 99;
    match f::unpack_ask(f::LAND, &buf) {
        Some(f::AskIn::Land { rule, .. }) => assert_eq!(rule, Rule::Public),
        other => panic!("该是 Land：{other:?}"),
    }

    // **旧帧**（尾格之前那些字节就够：入口有、两轴那两格没有）⇒ 两轴按"没有条件"读，
    // **不是读不懂**——帧加格子不该让旧调用方当场变坏。`mine` 读 `[50]`、`rule` 读 `[51]` 起。
    match f::unpack_ask(f::LAND, &buf[..tail_at + 8]) {
        Some(f::AskIn::Land { rule, mine, .. }) => {
            assert_eq!(rule, Rule::Public, "没有那一格 ⇒ 没有条件");
            assert!(!mine);
        }
        other => panic!("该是 Land：{other:?}"),
    }

    // 而**入口那一枚**是必须的：少一个字节就是读不懂（不猜）。
    assert_eq!(
        f::unpack_ask(f::LAND, &buf[..tail_at + 7]),
        None,
        "缺入口那枚 ⇒ 不猜"
    );
}

#[test]
fn the_list_answer_refuses_a_count_that_disagrees_with_the_frame() {
    let ids = [EntryId::new(2), EntryId::new(4), EntryId::new(6)];
    let mut buf = [0u8; f::REPLY_MAX];
    let n = f::pack_list(&mut buf, ids.iter().copied());
    assert_eq!(n, 2 + 3 * 8);
    let back = f::read_list(&buf[..n]).expect("读得回来");
    // 几枚 = 走一遍数出来（`Listing` 的读面只留 `iter` 这一格，见它的照实记）。
    assert_eq!(back.iter().count(), 3);
    assert_eq!(back.iter().collect::<Vec<_>>(), ids.to_vec());

    assert_eq!(f::read_list(&buf[..n - 1]), Err(f::BAD), "短一字节");
    let mut liar = buf;
    liar[1] = 4; // 说有四枚，可帧里只有三枚
    assert_eq!(f::read_list(&liar[..n]), Err(f::BAD), "条数说谎");
    // 状态那一格不是 `OK` ⇒ 原样把那一格报回去（读的人按它分流）。
    let mut failed = buf;
    failed[0] = f::FULL;
    assert_eq!(f::read_list(&failed[..n]), Err(f::FULL));
    assert_eq!(f::read_list(&[]), Err(f::BAD), "空帧");
}

#[test]
fn the_name_and_id_answers_are_fixed_shapes() {
    // **名那一形：长度即名长**（不是一个定长格 + 长度格）——故"多出来的"字节会被读成名字的一部分。
    let mut buf = [0u8; f::REPLY_MAX];
    let n = f::pack_name(&mut buf, Name::new("uart").unwrap());
    assert_eq!(n, 1 + 4);
    assert_eq!(f::read_name(&buf[..n]), Ok(Name::new("uart").unwrap()));

    // 多一个**零**：名字里夹 NUL ⇒ 判废；多一个**别的字节**：那就是另一个名字（读得出来）。
    assert_eq!(f::read_name(&buf[..n + 1]), Err(f::BAD), "夹了 NUL");
    let mut longer = buf;
    longer[n] = b'X';
    assert_eq!(
        f::read_name(&longer[..n + 1]),
        Ok(Name::new("uartX").unwrap())
    );

    // **号那一形是定长格**：短一字节、长一字节都是读不懂。
    // 照实记：`长一字节`这一句是牙口量出来的——把 `!= 8` 放宽成 `< 8` 之后，先前只测"短一字节"
    // 的那一版**全门照绿**。
    let mut buf = [0u8; f::REPLY_MAX];
    let n = f::pack_id(&mut buf, EntryId::new(12));
    assert_eq!(n, f::ID_REPLY_LEN);
    assert_eq!(f::read_id(&buf[..n]), Ok(EntryId::new(12)));
    assert_eq!(f::read_id(&buf[..n - 1]), Err(f::BAD), "短一字节");
    assert_eq!(f::read_id(&buf[..n + 1]), Err(f::BAD), "长一字节");
    let mut failed = buf;
    failed[0] = f::DEAD;
    assert_eq!(f::read_id(&failed[..n]), Err(f::DEAD), "状态那一格原样报回");
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
// `crates/protocol/src/system/operator/frame.rs` 的 `const _: () = assert!(…)` 里——**编译期**，
// riscv 那一档也一样钉着；比它强，且不再占一条用例。
