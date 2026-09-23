//! operator::judge —— **门外那一问**：这一位许不许动这一格。**不带载体、不碰内核。**
//!
//! 本文件与 [`core`](super::core) 同一站位：**只判规矩，不发消息**。判据要问的事实由
//! 调用方以四个 trait 注入（本文件因此既不认识会话、也不认识门牌）：
//!
//! ```text
//!   Who      TID ──→ 此刻代表哪条号            谁在问（名册，纵向）
//!   Branch   (号, 号) ──→ 在不在一支里          谱系谓词（纵向）
//!   League   盟 ──→ 这一位在不在这枚盟里        盟籍（横向）
//!   Door     格号 ──→ 开着那一格的那一位       树（**那一格是谁的门牌**）
//! ```
//!
//! `Door` 是唯一带**坐标**的一条：`Rule::Opens(e)` 比的是"开着第 `e` 格的那一位"，故它答的是
//! 一个 **TID**（内核戳"谁开的这扇门"），再拿那个 TID 回 [`Who`] 问"此刻代表谁"——两件事两个
//! 落点。
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
//! 划死：本文件只认识四样东西——`env::TaskId`、四个 trait、一个 [`Rule`]、以及 [`EntryId`]。
//! 调用点写 `judge::<PrincipalId, CoalitionId>`（或让它自己推），类型安全一分不减（两个号空间
//! 仍然互相排斥）。
//!
//! 照实记：这一格换过一次。第一版签名里写死了 `PrincipalId` / `CoalitionId`，代价是宿主靶要多编
//! 两个文件，而那两份文件今天的 `cfg(test)` 一次都没跑过——把新判据挂在没跑过的桩上，不划算。
//!
//! 照实记（第五格那一刀）：[`EntryId`] 是**同一个模块族**的核心（`super::core`），而 `judge-case`
//! 那一台**本来就编着** `core.rs`（账的两把钥匙就是 `Where` / `EntryId`）——故引它与引
//! `PrincipalId` 不是一回事，那条纪律一个口子都没开。

use env::TaskId;

use super::core::EntryId;

// ── 号在模型里的宽度 ────────────────────────────────────────

/// 号在模型里的宽度 —— **与它在自己号空间里的宽度一致**（riscv64：`usize` = 8 字节）。
///
/// 本文件与 [`gate`](super::gate) 只认识这一格别名，不认识 `PrincipalId` / `CoalitionId`
/// （那两个号是泛型的 `P` / `C`，见文件头注）。定死宽度是为了让**上帧的那一格**与这里的
/// 那一格同宽。
///
/// 照实记：这一格原先写的是 `u32`，而适配层接的是 `usize`——`Session::who` 那一处写着
/// `p.get() as u32`，一次**静默截断**。号不上帧的时候看不出来（装配期的号都是小号）；
/// 这一刀之后号要上帧（8 字节），故一并提宽。
pub type Id = u64;

// ── 一格规则 ────────────────────────────────────────────────

/// **这一格谁许用**。五格覆盖"公开 / 就是某一位 / 在某一位那一支里 / 在某枚盟里 /
/// 就是开着某一格的那一位"。
///
/// `By`（落牌那一位）**不进这一格**：规则改不改由它说了算（判据在适配层），而"谁能改规则"
/// 与"谁能用这一格"是两个问题——混成一格就会得出"能改的人自然能用"。
///
/// **`Opens` 那一格是"点名那一手"**：前四格只能指到"自己人"（自己的号、自己那一支、自己在的
/// 盟），而 `Opens` 指的是一格**门牌**——客人用 [`seek`](super::core::Operator::seek) 把一条路
/// 译成号，再把那个号写进规矩，于是「把这一格许给 `/device/uart` 那位」写得出来。名字由树
/// 提供（**树就是名录**），故规矩里存的是**格号**，不是身份号：判的那一刻才去问"此刻谁占着
/// 那一格"（晚绑定，与 [`Rule::In`] 同一形状——存一枚盟号，成员现场问）。
///
/// 照实记：**号不重用**（`core.rs` 只增水位）⇒ 那一格被剪/被顶之后，这一条规矩**永久判不了**
/// （重挂是**新号**）。这是"此刻占着这一格的那位"的题中之义，不是缺陷；要"换载体规矩不变"
/// 就得给身份起名字（那是另一条路，今天没有客人要它）。
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
    /// **就是开着第 `e` 格的那一位**（那一格的坐标是 [`EntryId`]，不是身份号）。
    Opens(EntryId),
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

/// 盟册那一侧：**`me` 此刻在这枚盟里吗**（横向）。
///
/// 与 [`Branch`] 分家（纵向 / 横向），实现上通常都是另一枚门牌。
///
/// **照实记：这一格原来少一个号。** 第一版写的是 `amid(&self, at: C)`——可盟册那一问本来
/// 就要两个号（`coalition::client::Face::amid(p, c)`，线上那一帧也带两格）。而 `judge` 手里
/// 明明有 `me`（第一步就问出来了），却没往下传。今天它是个恒答"问不到"的桩，故这个错一次
/// 没响过；通线（`Rule::In` 真跑）之前必须补上。
pub trait League<P, C> {
    fn amid(&self, me: P, at: C) -> Result<bool, ()>;
}

/// 树那一侧：**第 `at` 格是谁开的**（一格里那件事）。
///
/// - `Ok(Some(tid))` = 那一格是枚 `Tile`，开者是 `tid`；
/// - `Ok(None)` = **没有那一位**——碑 / 那一号是块 `Pane` / 开者那扇门封印了（**三因同落**：
///   判据只需要"有没有那一位"这一件事，三因分开是三个没人读的格）；
/// - `Err(())` = 树自己问不到（⇒ [`Ruling::Unjudged`]）。
///
/// 答的是 **TID 不是号**：树的读答"谁开的这扇门"（内核戳），名册那一边答"这个 TID 是谁"——
/// 两件事两个落点，故 `Rule::Opens` 那一格要走两问。
pub trait Door {
    fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()>;
}

// ── 那一问 ─────────────────────────────────────────────────

/// 判一格。`who` 是内核在 `Push` 那一刻盖的章；`rule` 是那一格自己的规矩。
///
/// **先问身份**——这一格是契约的一半：
///
/// - 先问：`who` 答不出就当场 [`Ruling::Deny`]，后面那几条边**一次都不发**（省一次 envcalls，
///   也让"没身份"与"不在支里"不会混成同一个答案）；
/// - **只问一次**——[`Rule::Opens`] 那一格是唯一的例外：它要**再问一次名册**，问的是**另一条
///   TID**（那一格的开者）"此刻代表谁"。这是晚绑定的代价，写在签名边上，不藏在实现里
///   （照实记：这一句原来写的是"只问一次"，一个字不含糊；加第五格时它被破了，故改口径）。
pub fn judge<P, C>(
    who: TaskId,
    rule: Rule<P, C>,
    roster: &impl Who<P>,
    branch: &impl Branch<P>,
    league: &impl League<P, C>,
    door: &impl Door,
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
        Rule::In(c) => match league.amid(me, c) {
            Ok(true) => Ruling::Allow,
            Ok(false) => Ruling::Deny,
            Err(()) => Ruling::Unjudged,
        },
        // 三、**两问**：先问树"那一格谁开着"，再问名册"那位此刻代表谁"。两问的失败域各自落格，
        //    与上面几条同一分法：**"没有那一位"是判不了**（可重试），**"那一位没身份"是终态拒**。
        Rule::Opens(at) => match door.opens(at) {
            Ok(Some(that)) => match roster.who(that) {
                Ok(Some(theirs)) => allow(me == theirs),
                Ok(None) => Ruling::Deny,
                Err(()) => Ruling::Unjudged,
            },
            Ok(None) => Ruling::Unjudged,
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
    const MATE: TaskId = TaskId::new(55);
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
    ///
    /// 照实记：这一格原来写的是 `contains(&P)`（拿常量当"我"），因为老签名根本收不到"谁在问"。
    /// 补上 `me` 之后它才真的在问"**这一位**在不在这枚盟里"。
    struct Book(&'static [u64]);
    impl League<u64, u64> for Book {
        fn amid(&self, me: u64, at: u64) -> Result<bool, ()> {
            Ok(at == C && self.0.contains(&me))
        }
    }

    /// 假树：一张 **格号 → 开者** 的表；表外的号答"没有那一位"（`Ok(None)`），与真树那三因同落。
    struct Doors(&'static [(usize, TaskId)]);
    impl Door for Doors {
        fn opens(&self, at: EntryId) -> Result<Option<TaskId>, ()> {
            Ok(self.0.iter().find(|(e, _)| *e == at.get()).map(|(_, t)| *t))
        }
    }

    /// 问不到的树。
    struct Deaf;
    impl Door for Deaf {
        fn opens(&self, _: EntryId) -> Result<Option<TaskId>, ()> {
            Err(())
        }
    }

    fn go(tid: TaskId, rule: Rule<u64, u64>) -> Ruling {
        let roster = Roster(&[(ME, P), (MATE, Q)]);
        let chain = Chain(&[(P, Q)]); // P 的父是 Q ⇒ Q ≼ P
        let book = Book(&[P]);
        // 第 5 格的门牌是 **MATE 开的** ⇒ `Opens(5)` 正是"许给那一位（不是我）"。
        let doors = Doors(&[(5, MATE)]);
        judge(tid, rule, &roster, &chain, &book, &doors)
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
    fn opens_is_about_who_holds_the_door_not_about_me() {
        // 第 5 格是 MATE 的门牌 ⇒ 它过；别人（有身份、但不是那一位）拒。
        assert_eq!(go(MATE, Rule::Opens(EntryId::new(5))), Ruling::Allow);
        assert_eq!(go(ME, Rule::Opens(EntryId::new(5))), Ruling::Deny);
        assert_eq!(go(OTHER, Rule::Opens(EntryId::new(5))), Ruling::Deny);
    }

    #[test]
    fn opens_without_such_a_door_is_unjudged() {
        // 「没有那一位」≠「你没资格」：格不在（碑 / 是块 Pane / 门封印）⇒ 判不了，可重试。
        assert_eq!(go(ME, Rule::Opens(EntryId::new(6))), Ruling::Unjudged);
        // 树自己问不到 ⇒ 同上。
        let roster = Roster(&[(ME, P)]);
        let chain = Chain(&[]);
        let book = Book(&[P]);
        assert_eq!(
            judge(
                ME,
                Rule::Opens(EntryId::new(5)),
                &roster,
                &chain,
                &book,
                &Deaf
            ),
            Ruling::Unjudged
        );
        // **但开者没绑身份 ⇒ 终态拒**（与"没身份就是没资格"同一条）。
        let doors = Doors(&[(5, UNBOUND)]);
        assert_eq!(
            judge(
                ME,
                Rule::Opens(EntryId::new(5)),
                &roster,
                &chain,
                &book,
                &doors
            ),
            Ruling::Deny
        );
    }

    #[test]
    fn an_unreachable_roster_is_unjudged_never_denied() {
        // 这一格钉的是"判不了 ≠ 你没资格"：把 Err 读成 Deny，整机就会把"身份服务挂了"
        // 报成"没权限"。
        let roster = Broken;
        let chain = Chain(&[]);
        let book = Book(&[P]);
        let doors = Doors(&[(5, MATE)]);
        assert_eq!(
            judge(ME, Rule::Is(P), &roster, &chain, &book, &doors),
            Ruling::Unjudged
        );
    }

    #[test]
    fn only_the_verb_is_asked_once_and_after_the_identity_gate() {
        // 顺序契约：没身份 ⇒ 后面那几条边一次都不发。喂一个"会记账的名册"就能看见这一点
        // （本格用 Broken 已经覆盖：`Err` 也走不到谓词那两条）。
        let roster = Roster(&[]);
        let chain = Chain(&[]);
        let book = Book(&[]);
        let doors = Deaf; // 会问到的树一律答"问不到"——这里要证的是"根本没问到它"
        assert_eq!(
            judge(ME, Rule::Under(P), &roster, &chain, &book, &doors),
            Ruling::Deny
        );
        // `Opens` 那一格是唯一的例外：**它要多问一次名册**（问的是开者那条 TID）。
        // 这里量的是"没身份 ⇒ 树那一问也不发"仍然成立（`doors = Deaf` 而不出 `Unjudged`）。
        assert_eq!(
            judge(
                ME,
                Rule::Opens(EntryId::new(5)),
                &roster,
                &chain,
                &book,
                &doors
            ),
            Ruling::Deny
        );
    }
}
