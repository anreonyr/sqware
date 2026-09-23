//! 门外那一问的门（**宿主台**）—— 裁决那三格规矩 + 它翻成线上那一格 + **那一本账**，
//! 在宿主上真跑一遍。
//!
//! # 这批判据钉的是什么
//!
//! `crates/protocol/src/operator/judge.rs` 是"这一位许不许动这一格"的**定义**；
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
//! 1. **三格答案各归各位**：`Allow` / `Deny`（终态）/ `Unjudged`（可重试）——若把"判不了"
//!    读成"你没资格"，整机就会把"身份服务挂了"报成"没权限"；
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
//! `operator/core.rs` 里。**多编一份它是挑过的**：`core.rs` 里**没有** `#[cfg(test)]`
//! （它的用例早搬去 `crates/operator-case` 了），故引它**不带进重复用例**；反过来把
//! `judge.rs` 引进 `operator-case` 会把上面这些**再跑一遍**。
//!
//! # 为什么这一台不编 `principal/core.rs`
//!
//! `judge.rs` 里两个号是**泛型**（`Rule<P, C>`），本台就用 `u64` 当那两个号（= [`Id`]，
//! 也是线上那一格的宽度）。这不是省事：若判据直接写死 `PrincipalId` / `CoalitionId`，本台
//! 就得跟着编那两份核心源码，而它们的 `#[cfg(test)]` 一次都没跑过——把新判据挂在没跑过的
//! 桩上不划算（照实记见 `judge.rs` 头注）。

extern crate alloc;

/// 判据的正文（就是 `crates/protocol/src/operator/judge.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/operator/judge.rs"]
mod judge;

/// 裁决 → 线上那一格（就是 `crates/protocol/src/operator/gate.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/operator/gate.rs"]
mod gate;

/// 树的正文（就是 `crates/protocol/src/operator/core.rs` 那一份，逐字未改）——**只为账要的
/// 那三样**（`Where` / `EntryId` / `Fail` / `VestedBy`），本台不测树本身。
// 照实记：`core.rs` 那一份**只借它的类型**（本台不测树本身）⇒ 它那一族方法在这一台里
// 全是"没被叫过"。挂 `allow(dead_code)` 而不是把那几条删掉：那是**逐字未改的源码**，
// 改它就是改判据的对象。同一个文件在 `crates/operator-case` 那一台里是被叫全的。
#[allow(dead_code)]
#[path = "../../protocol/src/operator/core.rs"]
mod core;

/// 那一本账（就是 `crates/protocol/src/operator/ledger.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/operator/ledger.rs"]
mod ledger;

use env::{Name, PieToken, TaskId};

use crate::core::{EntryId, Where};
use crate::gate::{Blind, Code, Control, verdict};
use crate::judge::{Branch, Door, Id, League, Rule, Ruling, Who, judge};
use crate::ledger::{Key, Ledger, Line};

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
    assert_eq!(go(OTHER, Rule::Public), Ruling::Allow, "别的已绑身份也算公开");
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
    assert_eq!(go(ME, Rule::In(4)), Ruling::Deny, "没铸过的盟：false，不是失败");
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
    // 三态各归各位：**"没有那一位"判不了**（挂门牌有先后，可重试）；
    // **"那一位没身份"是终态拒**（与第一条闸同一分法）；**树问不到**也是判不了。
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
    // **判不了 ≠ 你没资格**。这一条把"身份服务挂了/超时"与"你没有权限"分开——
    // 客人的下一步不同：前者重试，后者放弃。
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
        Ok([(ME, P), (OTHER, 11)].iter().find(|(t, _)| *t == tid).map(|(_, p)| *p))
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

#[test]
fn the_verdict_maps_onto_the_wire_cells() {
    // 放行 / 终态拒 / 判不了——三格各归各位（真值来自上面那三张假表）。
    assert_eq!(verdict(&Facts, ME, Rule::Public), Code::Ok);
    assert_eq!(verdict(&Facts, UNBOUND, Rule::Public), Code::Denied);
    assert_eq!(verdict(&Facts, ME, Rule::Under(Q)), Code::Ok);
    assert_eq!(verdict(&Facts, ME, Rule::In(C)), Code::Ok);
    assert_eq!(verdict(&Facts, ME, Rule::Under(11)), Code::Denied);
    // 第五格：那一格是 OTHER 的门牌 ⇒ 它过、ME 拒；"没有那一位"是判不了（不是拒）。
    assert_eq!(verdict(&Facts, OTHER, Rule::Opens(EntryId::new(5))), Code::Ok);
    assert_eq!(verdict(&Facts, ME, Rule::Opens(EntryId::new(5))), Code::Denied);
    assert_eq!(
        verdict(&Facts, ME, Rule::Opens(EntryId::new(6))),
        Code::Unjudged
    );
}

#[test]
fn no_face_at_all_is_blind_and_never_allow() {
    // **装配期**：树手里还没有协调门牌 ⇒ 判不了任何人。这一格**不是放行**——
    // 松成放行就等于"协调服务没配上 ⇒ 门禁不存在"。
    assert_eq!(verdict(&Blind, ME, Rule::Public), Code::Blind);
    assert!(!verdict(&Blind, ME, Rule::Public).passed());
}

// ── 那一本账（ledger 那一层）────────────────────────────────
//
// 这四组钉的是账的三条契约：**两种寻址打的是同一格**、**陈旧那一行答默认 + 顺手销账**、
// **"活着"那一问就是主人还在不在场**、**先要位才写得进**。
//
// 照实记（为什么这批判据值得写）：账的前身（`server.rs` 的 `Publishers`）住在适配层，
// **编不进宿主靶**——而它在真机上量错过两次（第一版按号记 ⇒ 整道判据被跳过，`probe-owner`
// 当场顶掉了 `/device/uart` 的牌子）。这是它第一次被逐字编进测试靶。

/// 造一枚号给账用：**唯一的门是"收号"**（与 `operator-case` 那一台同一条路）。
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
    book.write(blank, Line::new(AT, name("uart"), ID, rule, mine, ME, pie(n)));
    book
}

#[test]
fn an_empty_ledger_answers_public_and_free() {
    // **两条"没这一格"都不是失败**：`part` 出来的格子本来就没有规矩；占了它的人也没声明归属。
    let mut book: Ledger<Id, Id> = Ledger::new(vested_by);
    let truth = Truth::new(&[ID]);
    assert_eq!(book.rule(Key::Id(ID), |id| truth.fresh(id)), Rule::Public);
    assert_eq!(book.rule(Key::At(AT, name("uart")), |id| truth.fresh(id)), Rule::Public);
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
    assert!(book.claimable(Key::At(AT, name("uart")), ME, |id| truth.fresh(id)), "主人本人");
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
    book.write(blank, Line::new(AT, name("uart"), ID, Rule::Is(P), false, ME, pie(2)));
    assert_eq!(book.len(), 1, "重绑不该多出一条");
    assert_eq!(book.rule(Key::At(AT, name("uart")), |id| truth.fresh(id)), Rule::Is(P));
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
