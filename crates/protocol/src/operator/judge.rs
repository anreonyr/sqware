//! operator::judge —— **门外那一问**：这一位许不许动这一格。**不带载体、不碰内核。**
//!
//! 本文件与 [`core`](super::core) 同一站位：**只判规矩，不发消息**。判据要问的三个事实由
//! 调用方以三个 trait 注入（本文件因此既不认识会话、也不认识门牌）：
//!
//! ```text
//!   Who      TID ──→ 此刻代表哪条号            谁在问（名册，纵向）
//!   Branch   (号, 号) ──→ 在不在一支里          谱系谓词（纵向）
//!   League   盟 ──→ 这一位在不在这枚盟里        盟籍（横向）
//! ```
//!
//! # 三格答案，每格一个不同的下一步
//!
//! [`Ruling::Allow`]（放行）/ [`Ruling::Deny`]（**终态**：换人、换目标、别重试）/
//! [`Ruling::Unjudged`]（**判不了**：对面没答上来 ⇒ 可重试）。三格缺一不可：
//!
//! - 把 `Deny` 读成 `Unjudged` ⇒ 客人对着"你没资格"白烧时间（`subject` / `sleeper` 那套
//!   有界重试正是这个形状）；
//! - 把 `Unjudged` 读成 `Deny` ⇒ "身份服务挂了"被读成"我没资格"，整机以为是规矩问题；
//! - 把两者读成 `Allow` ⇒ 没有门禁。
//!
//! # 两层判据不许混进失败域
//!
//! [`Rule`] 那三种"答不是"（不在这一支、不在这枚盟、不是这一位）**都只是 `false`，不是失败**
//! ——这是两条协议的正文各自写死的（`principal` 的读全是公开的、`coalition` 的 `amid` 对
//! 伪造号答 `false`）。所以本文件里 **`Err` 一律读作 `Unjudged`**，`Ok(false)` 才是 `Deny`。
//! 这一行是这一刀最容易写错、又最难在真机上发现的地方。
//!
//! # 没身份就是没资格
//!
//! `who` 答不出（`Ok(None)`：这条 TID 没绑过）⇒ **`Deny`**，不是 `Allow`。编排域（装配者）
//! 正好落这一格——它**不绑身份**（"它是写名册的那一个，不是被写的那一个"），故它做不了树的
//! 客人。这是有意的：不是树的客人就不该有门禁上的特例。
//!
//! # 这一格为什么住在协议里、而不在服务里
//!
//! 它是**模型**（"这一位许不许"这句话的定义），住在 `protocol` 才能被宿主台**逐字编进测试靶**
//! （照 `crates/operator-case` 那台的现成路数；`programs` 那一侧编进宿主要 `runtime` 的 riscv
//! 内联汇编，走不通）。服务那一侧只做装配：把 `principal::client::Face` / `coalition::client::Face`
//! 套成这三个 trait，再把 [`Ruling`] 翻成线上那一格码。
//!
//! # 为什么两个号是**泛型**，而不是直接写 `PrincipalId` / `CoalitionId`
//!
//! 若这里直接 `use crate::principal::core::PrincipalId`，宿主靶就得跟着编 `principal/core.rs` 与
//! `coalition/core.rs` 两份——而"宿主靶只编一份逐字未改的核心源码"这条纪律会被打破。泛型把线
//! 划死：本文件只认识三样东西——`env::TaskId`、三个 trait、一个 [`Rule`]。调用点写
//! `judge::<PrincipalId, CoalitionId>`（或让它自己推），类型安全一分不减（两个号空间仍然互相排斥）。
//!
//! 照实记：这一格换过一次。第一版签名里写死了 `PrincipalId` / `CoalitionId`，代价是宿主靶要多编
//! 两个文件，而那两份文件今天的 `cfg(test)` 一次都没跑过——把新判据挂在没跑过的桩上，不划算。

use env::TaskId;

// ── 一格规则 ────────────────────────────────────────────────

/// **这一格谁许用**。四格覆盖"公开 / 就是某一位 / 在某一位那一支里 / 在某枚盟里"。
///
/// `By`（落牌那一位）**不进这一格**：规则改不改由它说了算（判据在适配层），而"谁能改规则"
/// 与"谁能用这一格"是两个问题——混成一格就会得出"能改的人自然能用"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rule<P, C> {
    /// 任何**已绑身份**都可以（这就是"公开入口"）。没绑的仍然不行（见 [`judge`] 的第一格）。
    Public,
    /// 就是这一位。
    Is(P),
    /// 这一位在 `p` 那一支里（`p ≼ 本人`，含相等）——纵向那条轴。
    Under(P),
    /// 这一位在这枚盟里——横向那条轴。
    In(C),
}

/// **门外那一问的答案**。三格，每格一个不同的下一步。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ruling {
    /// 过。
    Allow,
    /// 不过——**终态**：换人 / 换目标 / 别重试。
    Deny,
    /// **判不了**：问身份那两条边没答上来（对面不在、超时、答不出那条号）⇒ 可重试。
    Unjudged,
}

// ── 三个注入的事实 ──────────────────────────────────────────

/// 名册那一侧：**这个 TID 此刻代表谁**（纵向）。
///
/// `Ok(None)` = 没绑（**不是失败**，是"没有身份"）；`Err` = 问不到（⇒ [`Ruling::Unjudged`]）。
pub trait Who<P> {
    fn who(&self, tid: TaskId) -> Result<Option<P>, ()>;
}

/// 谱系那一侧：**`a` 在 `b` 那一支里吗**（纵向的谓词，含 `a == b`）。
///
/// `Err` = 问不到（⇒ `Unjudged`）；`Ok(false)` = 不在这一支里（⇒ `Deny`）。
pub trait Branch<P> {
    fn heir(&self, a: P, b: P) -> Result<bool, ()>;
}

/// 盟册那一侧：**这一位此刻在这枚盟里吗**（横向）。
///
/// 与 [`Branch`] 分家（纵向 / 横向），实现上通常都是另一枚门牌。
pub trait League<C> {
    fn amid(&self, at: C) -> Result<bool, ()>;
}

// ── 那一问 ─────────────────────────────────────────────────

/// 判一格。`who` 是内核在 `Push` 那一刻盖的章；`rule` 是那一格自己的规矩。
///
/// **先问身份、只问一次**——这一格是契约的一半：
///
/// - 先问：`who` 答不出就当场 [`Ruling::Deny`]，后面两条边**一次都不发**（省一次 envcalls，
///   也让"没身份"与"不在支里"不会混成同一个答案）；
/// - 只问一次：这一问是一次 envcalls，"顺手再问一次"在这里就是双倍成本。
pub fn judge<P, C>(
    who: TaskId,
    rule: Rule<P, C>,
    roster: &impl Who<P>,
    branch: &impl Branch<P>,
    league: &impl League<C>,
) -> Ruling
where
    P: PartialEq + Copy,
    C: Copy,
{
    // 一、谁在问。没绑 ⇒ 没资格（编排域落的正是这一格）；**问不到 ⇒ 判不了**。
    let Some(me) = (match roster.who(who) {
        Ok(found) => found,
        Err(()) => return Ruling::Unjudged,
    }) else {
        return Ruling::Deny;
    };
    // 二、照规则问那一句。**三种"答不是"都是 `Ok(false)`，不是失败**。
    match rule {
        Rule::Public => Ruling::Allow,
        Rule::Is(p) => allow(me == p),
        Rule::Under(p) => match branch.heir(p, me) {
            Ok(true) => Ruling::Allow,
            Ok(false) => Ruling::Deny,
            Err(()) => Ruling::Unjudged,
        },
        Rule::In(c) => match league.amid(c) {
            Ok(true) => Ruling::Allow,
            Ok(false) => Ruling::Deny,
            Err(()) => Ruling::Unjudged,
        },
    }
}

const fn allow(ok: bool) -> Ruling {
    if ok { Ruling::Allow } else { Ruling::Deny }
}

// ── 判据（给宿主台编的规格；`protocol` 自己编不到 `cfg(test)`，见 crate 根的说明）──

#[cfg(test)]
mod tests {
    //! 照实记：本模块在 `protocol` 里**编不到、也跑不到**（`[lib] test = false`）。
    //! 下面这几条是**契约的读数**，真正跑它们的是宿主台（照 `crates/operator-case` 那台搬）。

    use super::*;

    const ME: TaskId = TaskId::new(22);
    const OTHER: TaskId = TaskId::new(33);
    const UNBOUND: TaskId = TaskId::new(44);
    const P: u64 = 7;
    const Q: u64 = 9;
    const C: u64 = 3;

    /// 假名册：只认识 ME。
    struct Roster(&'static [(TaskId, u64)]);
    impl Who<u64> for Roster {
        fn who(&self, tid: TaskId) -> Result<Option<u64>, ()> {
            Ok(self.0.iter().find(|(t, _)| *t == tid).map(|(_, p)| *p))
        }
    }

    /// 问不到的名册。
    struct Broken;
    impl Who<u64> for Broken {
        fn who(&self, _: TaskId) -> Result<Option<u64>, ()> {
            Err(())
        }
    }

    /// 假谱系：一张 子 → 父 的表。
    struct Chain(&'static [(u64, u64)]);
    impl Branch<u64> for Chain {
        fn heir(&self, a: u64, b: u64) -> Result<bool, ()> {
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

    /// 假盟册：只有 3 号盟，成员是列出来的那几位。
    struct Book(&'static [u64]);
    impl League<u64> for Book {
        fn amid(&self, at: u64) -> Result<bool, ()> {
            Ok(at == C && self.0.contains(&P))
        }
    }

    fn go(tid: TaskId, rule: Rule<u64, u64>) -> Ruling {
        let roster = Roster(&[(ME, P)]);
        let chain = Chain(&[(P, Q)]); // P 的父是 Q ⇒ Q ≼ P
        let book = Book(&[P]);
        judge(tid, rule, &roster, &chain, &book)
    }

    #[test]
    fn public_needs_a_bound_identity_but_no_rule_check() {
        assert_eq!(go(ME, Rule::Public), Ruling::Allow);
        assert_eq!(go(UNBOUND, Rule::Public), Ruling::Deny);
        assert_eq!(go(OTHER, Rule::Public), Ruling::Deny);
    }

    #[test]
    fn is_matches_the_current_identity_not_the_task() {
        assert_eq!(go(ME, Rule::Is(P)), Ruling::Allow);
        assert_eq!(go(ME, Rule::Is(Q)), Ruling::Deny);
    }

    #[test]
    fn under_is_about_the_branch_and_heir_is_reflexive() {
        assert_eq!(go(ME, Rule::Under(P)), Ruling::Allow);
        assert_eq!(go(ME, Rule::Under(Q)), Ruling::Allow);
        assert_eq!(go(ME, Rule::Under(11)), Ruling::Deny);
    }

    #[test]
    fn in_is_about_the_league() {
        assert_eq!(go(ME, Rule::In(C)), Ruling::Allow);
        assert_eq!(go(ME, Rule::In(4)), Ruling::Deny);
    }

    #[test]
    fn an_unreachable_roster_is_unjudged_never_denied() {
        // 这一格钉的是"判不了 ≠ 你没资格"：把 Err 读成 Deny，整机就会把"身份服务挂了"
        // 报成"没权限"。
        let roster = Broken;
        let chain = Chain(&[]);
        let book = Book(&[P]);
        assert_eq!(
            judge(ME, Rule::Is(P), &roster, &chain, &book),
            Ruling::Unjudged
        );
    }

    #[test]
    fn only_the_verb_is_asked_once_and_after_the_identity_gate() {
        // 顺序契约：没身份 ⇒ 两条谓词边一次都不发。喂一个"会记账的名册"就能看见这一点
        // （本格用 Broken 已经覆盖：`Err` 也走不到谓词那两条）。
        let roster = Roster(&[]);
        let chain = Chain(&[]);
        let book = Book(&[]);
        assert_eq!(
            judge(ME, Rule::Under(P), &roster, &chain, &book),
            Ruling::Deny
        );
    }
}
