//! 门外那一问的门（**宿主台**）—— 裁决那三格规矩 + 它翻成线上那一格，在宿主上真跑一遍。
//!
//! # 这批判据钉的是什么
//!
//! `crates/protocol/src/operator/judge.rs` 是"这一位许不许动这一格"的**定义**；
//! `crates/protocol/src/operator/gate.rs` 是它**翻成线上那一格**的那一层。两份都一行不发消息，
//! 故喂假事实就能把规矩推理干净：
//!
//! ```text
//!   Who      谁在问        ← 假名册（一张 TID → 号 的表）
//!   Branch   在不在一支里   ← 假谱系（一张 子 → 父 的表）
//!   League   在不在这枚盟里 ← 假盟册（一张成员表）
//!   Control  上面三条的出口 ← gate 那一侧的粘合（本台与树域各一份实现）
//! ```
//!
//! 三条判据、按重要性排：
//!
//! 1. **三格答案各归各位**：`Allow` / `Deny`（终态）/ `Unjudged`（可重试）——若把"判不了"
//!    读成"你没资格"，整机就会把"身份服务挂了"报成"没权限"；
//! 2. **没身份就是没资格**：没绑过的 TID 在 `Rule::Public` 上也要被拒（编排域落的正是这一格）
//!    ——这一条松掉，门禁就成了一条"不绑身份即可绕过"的后门；
//! 3. **`Ok(false)` 与 `Err` 分家**：前者是"不在"，后者是"问不到"；再加上「手里没有门牌」
//!    那一格（[`Code::Blind`]）——它**不是放行**。
//!
//! # 为什么这一台不编 `principal/core.rs`
//!
//! `judge.rs` 里两个号是**泛型**（`Rule<P, C>`），本台就用 `u32` 当那两个号。这不是省事：
//! 若判据直接写死 `PrincipalId` / `CoalitionId`，本台就得跟着编那两份核心源码，而它们的
//! `#[cfg(test)]` 一次都没跑过——把新判据挂在没跑过的桩上不划算（照实记见 `judge.rs` 头注）。

extern crate alloc;

/// 判据的正文（就是 `crates/protocol/src/operator/judge.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/operator/judge.rs"]
mod judge;

/// 裁决 → 线上那一格（就是 `crates/protocol/src/operator/gate.rs` 那一份，逐字未改）。
#[path = "../../protocol/src/operator/gate.rs"]
mod gate;

use env::TaskId;

use crate::gate::{Blind, Code, Control, verdict};
use crate::judge::{Branch, League, Rule, Ruling, Who, judge};

// ── 假事实 ──────────────────────────────────────────────────

/// 假名册：只有这一位绑过（号 = `7`）。
struct Roster(&'static [(TaskId, u32)]);
impl Who<u32> for Roster {
    fn who(&self, tid: TaskId) -> Result<Option<u32>, ()> {
        Ok(self.0.iter().find(|(t, _)| *t == tid).map(|(_, p)| *p))
    }
}

/// 假名册：**问不到**（身份服务不在 / 超时）。
struct Broken;
impl Who<u32> for Broken {
    fn who(&self, _: TaskId) -> Result<Option<u32>, ()> {
        Err(())
    }
}

/// 假谱系：一张 子 → 父 的表（走到头 = 根）。
struct Chain(&'static [(u32, u32)]);
impl Branch<u32> for Chain {
    fn heir(&self, a: u32, b: u32) -> Result<bool, ()> {
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
struct Book(&'static [u32]);
impl League<u32> for Book {
    fn amid(&self, at: u32) -> Result<bool, ()> {
        Ok(at == C && self.0.contains(&P))
    }
}

// ── 三个身份 / 一个不存在的 TID ─────────────────────────────

const ME: TaskId = TaskId::new(22); // 绑在号 7 上
const OTHER: TaskId = TaskId::new(33); // 绑在别的号上
const UNBOUND: TaskId = TaskId::new(44); // 从没绑过

const P: u32 = 7; // ME 的号
const Q: u32 = 9; // P 的父（故 Q ≼ P）
const C: u32 = 3; // 那枚盟

fn go(tid: TaskId, rule: Rule<u32, u32>) -> Ruling {
    let roster = Roster(&[(ME, P), (OTHER, 11)]);
    let chain = Chain(&[(P, Q)]); // P 的父是 Q ⇒ heir(Q, P) = true
    let book = Book(&[P]);
    judge(tid, rule, &roster, &chain, &book)
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
fn an_unreachable_roster_is_unjudged_never_denied() {
    // **判不了 ≠ 你没资格**。这一条把"身份服务挂了/超时"与"你没有权限"分开——
    // 客人的下一步不同：前者重试，后者放弃。
    let roster = Broken;
    let chain = Chain(&[]);
    let book = Book(&[P]);
    for rule in [Rule::Public, Rule::Is(P), Rule::Under(P), Rule::In(C)] {
        // `Public` 也要先问身份 ⇒ 问不到同样是"判不了"，不是"过"。
        assert_eq!(
            judge(ME, rule, &roster, &chain, &book),
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
        judge(UNBOUND, Rule::Under(P), &roster, &chain, &book),
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
impl Branch<u32> for Counting {
    fn heir(&self, _: u32, _: u32) -> Result<bool, ()> {
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
impl League<u32> for Counting2 {
    fn amid(&self, _: u32) -> Result<bool, ()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(false)
    }
}

// ── 裁决 → 线上那一格（gate 那一层）────────────────────────

/// 把本台那三个假事实收成一个 [`Control`]——与树域那一侧的真实现同形（那是两枚门牌）。
struct Facts;
impl Control for Facts {
    fn who(&self, tid: TaskId) -> Result<Option<u32>, ()> {
        Ok([(ME, P), (OTHER, 11)].iter().find(|(t, _)| *t == tid).map(|(_, p)| *p))
    }
    fn heir(&self, a: u32, b: u32) -> Result<bool, ()> {
        let mut at = Some(b);
        while let Some(cur) = at {
            if cur == a {
                return Ok(true);
            }
            at = [(P, Q)].iter().find(|(k, _)| *k == cur).map(|(_, v)| *v);
        }
        Ok(false)
    }
    fn amid(&self, at: u32) -> Result<bool, ()> {
        Ok(at == C && P == P)
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
}

#[test]
fn no_face_at_all_is_blind_and_never_allow() {
    // **装配期**：树手里还没有协调门牌 ⇒ 判不了任何人。这一格**不是放行**——
    // 松成放行就等于"协调服务没配上 ⇒ 门禁不存在"。
    assert_eq!(verdict(&Blind, ME, Rule::Public), Code::Blind);
    assert!(!verdict(&Blind, ME, Rule::Public).passed());
}
