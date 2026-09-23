//! operator::gate —— **裁决 → 线上那一格**：怎么问（[`Control`]）、怎么翻（[`verdict`]）。
//!
//! 本文件与 [`judge`](super::judge) 同一站位：**不带载体**。它只做两件事——
//!
//! 1. 把"问身份那两条边"收成**一个 trait**（[`Control`]）：树域那一侧用一个真实现
//!    （两枚门牌 + `Face::resolve` / `heir` / `amid`），宿主台那一侧用假实现；
//! 2. 把 [`judge`] 那三格答案 + "手里没有门牌"这一格，翻成线上那一格码（[`Code`]）。
//!
//! # 四种码，四种下一步
//!
//! ```text
//!   Code::Ok        放行（继续去动树）
//!   Code::Denied    「这一位不许」——**终态**：换人 / 换目标，别重试
//!   Code::Unjudged  「现在判不了」——对面没答上来，可重试
//!   Code::Blind     「本域手里还没有门牌」——装配期的那一段（**不是**"放行"）
//! ```
//!
//! [`Code::Blind`] 单列的理由：它是**装配期的定义**，不是运行期的例外。树在那一段里还没有
//! 协调门牌，判不了任何人；若把这一段读成"放行"，就等于"协调服务没配上 ⇒ 门禁不存在"。
//! 它翻成 [`UNJUDGED`](crate::operator::call::UNJUDGED)，客人的下一步与"判不了"相同（可重试）
//! ——但树侧的读数要能把它与"问了但对面没答"分开（装配诊断靠这一句）。
//!
//! # 默认策略 = 公开
//!
//! [`Rule::Public`] 的含义是"**任何已绑身份都可以**"——没绑的仍然不行（那是 [`judge`] 的第一格）。
//! 它今天是**"这一格没记过规矩"那一条**的答案（逐格规矩住在 [`ledger`](super::ledger) 里，
//! 见 [`Ledger::rule`](super::ledger::Ledger::rule)）：`part` 分出来的格子本来就没有规矩。
//! **默认必须是它**，否则既有的 11 条 `tree part=0 … land=0 find=0 got=true` 会当场塌。
//!
//! 照实记：上一刀这里写的是"条目上还没有逐格规则（那是下一刀）"——那一刀所有条目共用这一条
//! 常量，故 `judge` 里 `Is` / `Under` / `In` 三条判据**一次没被问过**；这一刀通了它们。

use super::judge::{Branch, Id, League, Rule, Ruling, Who, judge};
use env::TaskId;

// ── 线上那一格：**本文件自己拿一份** ────────────────────────
//
// 照实记：这里**不 `use super::call::*`**。理由是宿主台——本文件要与 `judge.rs` 一起被
// `#[path]` 编进测试靶，而 `call.rs` 拖着 `runtime`（门闩那一族）与 `session`，宿主上编不动。
// 故这三格在本文件里各留一个常量，**同步义务由 `mod.rs` 末尾那条 `const _: () = assert!(…)`
// 在编译期钉住**：真正的对照表只有一份（`call.rs`），这里这一份只要一漂就编不过。
pub(crate) const WIRE_OK: u8 = 0;
pub(crate) const WIRE_DENIED: u8 = 8;
pub(crate) const WIRE_UNJUDGED: u8 = 9;

/// 一颗线上码——裁决那一侧的全部出口。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// 放行（继续去动树）。
    Ok,
    /// 这一位不许。**终态**。
    Denied,
    /// 现在判不了（对面没答上来）。可重试。
    Unjudged,
    /// 本域手里还没有门牌（装配期）。可重试，但读数与上一条分开。
    Blind,
}

impl Code {
    /// 写给客人的那一格。
    pub const fn wire(self) -> u8 {
        match self {
            Code::Ok => WIRE_OK,
            Code::Denied => WIRE_DENIED,
            Code::Unjudged | Code::Blind => WIRE_UNJUDGED,
        }
    }

    /// 放行了吗——调用点只该问这一句。
    pub const fn passed(self) -> bool {
        matches!(self, Code::Ok)
    }
}

/// 问身份那两条边的**一个**出口：树域用它接两枚门牌，宿主台用它喂假事实。
pub trait Control {
    /// **手里有没有协调门牌**。装配期（还没有门牌）答 `false` ⇒ 一律 [`Code::Blind`]。
    ///
    /// 单列这一格的道理：`judge` 分不出"问不到"是"对面没答"还是"我压根没门牌问"——而这两件事
    /// 在装配诊断上不是一回事。默认 `true`（有门牌是实现这一 trait 的前提）。
    fn has_face(&self) -> bool {
        true
    }
    /// 这个 TID 此刻代表哪条号（`Ok(None)` = 没绑；`Err` = 问不到）。
    fn who(&self, tid: TaskId) -> Result<Option<Id>, ()>;
    /// `a` 在 `b` 那一支里吗（`Err` = 问不到）。
    fn heir(&self, a: Id, b: Id) -> Result<bool, ()>;
    /// **`me`** 在这一枚盟里吗（`Err` = 问不到）。两格都要：盟册那一问本来就是这个形状。
    fn amid(&self, me: Id, at: Id) -> Result<bool, ()>;
}

/// **还没有门牌**的那一份实现：`has_face` 答 `false`，其余三问一律答"问不到"。
///
/// 它就是 [`Code::Blind`] 的来源——装配期（树手里没有协调门牌）用的正是它。
pub struct Blind;

impl Control for Blind {
    fn has_face(&self) -> bool {
        false
    }
    fn who(&self, _: TaskId) -> Result<Option<Id>, ()> {
        Err(())
    }
    fn heir(&self, _: Id, _: Id) -> Result<bool, ()> {
        Err(())
    }
    fn amid(&self, _: Id, _: Id) -> Result<bool, ()> {
        Err(())
    }
}

/// 三个假事实拼成 [`judge`] 要的那三个 trait——**本文件的全部粘合**。
struct Facts<'a, C: Control>(&'a C);

impl<C: Control> Who<Id> for Facts<'_, C> {
    fn who(&self, tid: TaskId) -> Result<Option<Id>, ()> {
        self.0.who(tid)
    }
}

impl<C: Control> Branch<Id> for Facts<'_, C> {
    fn heir(&self, a: Id, b: Id) -> Result<bool, ()> {
        self.0.heir(a, b)
    }
}

impl<C: Control> League<Id, Id> for Facts<'_, C> {
    fn amid(&self, me: Id, at: Id) -> Result<bool, ()> {
        self.0.amid(me, at)
    }
}

/// 判一格：`control` 是问身份那两条边（装配期喂 [`Blind`]），`rule` 是那一格自己的规矩。
pub fn verdict(control: &impl Control, who: TaskId, rule: Rule<Id, Id>) -> Code {
    // 手里没有门牌 ⇒ 当场答"装配期"，一次问都不发（省一次注定失败的 envcalls）。
    if !control.has_face() {
        return Code::Blind;
    }
    let facts = Facts(control);
    match judge(who, rule, &facts, &facts, &facts) {
        Ruling::Allow => Code::Ok,
        Ruling::Deny => Code::Denied,
        Ruling::Unjudged => Code::Unjudged,
    }
}

// ── 判据（给宿主台编的规格；`protocol` 自己编不到 `cfg(test)`）──

#[cfg(test)]
mod tests {
    // 线上那一格的值从本模块（父）继承——那三个常量住模块顶层，正是为了宿主靶也拿得到。
    use super::*;

    const ME: TaskId = TaskId::new(22);
    const P: Id = 7;

    /// 一颗能答的身份边。
    struct Real;
    impl Control for Real {
        fn who(&self, tid: TaskId) -> Result<Option<Id>, ()> {
            Ok((tid == ME).then_some(P))
        }
        fn heir(&self, a: Id, b: Id) -> Result<bool, ()> {
            Ok(a == b)
        }
        fn amid(&self, me: Id, at: Id) -> Result<bool, ()> {
            Ok(me == P && at == P)
        }
    }

    /// 一颗**答不上来**的身份边（服务在，但这一问没答）。
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
    }

    #[test]
    fn the_three_codes_map_to_the_three_wire_cells() {
        assert_eq!(Code::Ok.wire(), WIRE_OK);
        assert_eq!(Code::Denied.wire(), WIRE_DENIED);
        assert_eq!(Code::Unjudged.wire(), WIRE_UNJUDGED);
        // 「手里没有门牌」在客人那侧与「判不了」同一格（都可重试）——但树侧的读数分开。
        assert_eq!(Code::Blind.wire(), WIRE_UNJUDGED);
    }

    #[test]
    fn public_passes_for_a_bound_identity() {
        assert_eq!(verdict(&Real, ME, Rule::Public), Code::Ok);
    }

    #[test]
    fn a_missing_identity_is_denied_not_unjudged() {
        // 没绑过（名册答 `None`）⇒ **终态拒**：这一格若松成"判不了"，客人会一直重试。
        assert_eq!(verdict(&Real, TaskId::new(99), Rule::Public), Code::Denied);
    }

    #[test]
    fn a_mute_identity_service_is_unjudged_not_denied() {
        // 这一格钉的是"服务挂了"与"你没资格"分家。
        assert_eq!(verdict(&Mute, ME, Rule::Public), Code::Unjudged);
    }

    #[test]
    fn the_blind_control_is_blind() {
        // 装配期（树手里没有门牌）：判不了任何人——但**它不是"放行"**。
        assert_eq!(verdict(&Blind, ME, Rule::Public), Code::Blind);
        assert_eq!(verdict(&Blind, ME, Rule::Under(P)), Code::Blind);
        assert!(!Code::Blind.passed(), "没有门牌 ≠ 放行");
    }
}
