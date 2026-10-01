//! operator::core::gate —— **裁决 → 线上那一格**：怎么问（[`Facts`]）、怎么翻（[`verdict`]）。
//!
//! 本文件与 [`judge`](super::judge) 同一站位：**不带载体**。它只做一件事——
//! 把 [`judge`] 那三格答案翻成线上那一格码（[`Code`]）。判据要问的那几条边由树域那一侧
//! 实现 [`Facts`]（两枚门牌 + `Face::resolve` / `heir` / `amid`，外加**树自己**那条"这一格是
//! 谁的门牌"）；`Facts` 的定义在 `judge.rs`——它是 `judge` 直接问的那四条边，本文件不再转发。
//!
//! # 三种码，三种下一步
//!
//! ```text
//!   Code::Ok        放行（继续去动树）
//!   Code::Denied    「这一位不许」——**终态**：换人 / 换目标，别重试
//!   Code::Unjudged  「判不了」——这一问要的那条事实问不到（因分两类，见下）
//! ```
//!
//! **`Code::Unjudged` 不承诺"等一会儿会好"**：它的因有两类——**会好的**（对面不答 / 超时、
//! 门牌还没到）与**好不了的**（那一号是碑、那一格是块窗格、开者那扇门封印了）。两类在客人
//! 那一侧是同一个下一步（当趟放弃），差别只在**为什么** ⇒ 那是**读数**的格。「重试」是
//! **客人的策略**，不是这一格的承诺。
//!
//! **照实记（第四格 `Code::Blind` 已退场）**：它表达的是"本域手里还没有协调门牌"这个模型
//! 状态，读者只有已删的宿主靶；生产路径**到不了它**——树域在 `session == None` 时由 `may`
//! （`programs/src/system/operator/server.rs`）**在更早处短路**（理由照实记在那个函数自己的
//! 头注里：装配期 principal 挂自己门牌那一趟既没有门牌、又还没有身份，门禁若在那一刻生效，
//! 整机起不来）。那条短路还在，故这一格连"模型里的一格"都不是了 ⇒ 连同它的来源 `Blind`
//! 一起删。
//!
//! # 默认策略 = 没有许可
//!
//! [`Permit::Unset`] 的含义是"**这一格没记许可**"——判据只要求"有身份"（没绑的仍然不行，那是
//! [`judge`] 的第一格）。它今天是**"这一格没记过"那一条**的答案（许可跟着那一枚砖走，
//! 见 [`Operator::permit`](super::Operator::permit)）：`part` 分出来的格子本来就没有。
//! **默认必须是它**，否则既有的 11 条 `tree part=0 … land=0 find=0 got=true` 会当场塌。
//!
//! 照实记：上一刀这里写的是"条目上还没有逐格规则（那是下一刀）"——那一刀所有条目共用这一条
//! 常量，故 `judge` 里 `Trunk` / `Bough` / `Among` 三条判据**一次没被问过**；后一刀通了它们。

use env::TaskId;

use protocol::service::operator::{Permit, Ruling};

use super::judge::{Facts, judge};

// ── 线上那一格：**本文件自己拿一份** ────────────────────────
//
// 照实记：这里**不 `use` frame 那一份**。转发表住 `protocol::service::operator::frame`
// （它拖着 `message` 与那一族的帧），而本文件**不带载体**、只认 `env` 与 `judge`
// ——两条依赖面有意不同，故这一份不伸手过去拿。
//
// **照实记（同步断言搬到本侧了）**：这三格与 `frame` 那三格的同步义务由
// [`super`](crate::system::operator::core) 末尾那条 `const _: () = assert!(…)` 在编译期钉住
// ——三份文件原先分住两个 crate（断言只能在 protocol 那一侧做），搬回同一侧之后
// **同 crate 同见**，比原先更紧。
pub const WIRE_OK: u8 = 0;
pub const WIRE_DENIED: u8 = 8;
pub const WIRE_UNJUDGED: u8 = 9;

/// 一颗线上码——裁决那一侧的全部出口。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    /// 放行（继续去动树）。
    Ok,
    /// 这一位不许。**终态**。
    Denied,
    /// 判不了：这一问要的那条事实问不到——对面不答 / 超时（**会好**），或那一号是碑 /
    /// 那一格是块窗格 / 开者那扇门封印了（**好不了**）。两类同格；**重试是客人的策略**。
    Unjudged,
}

impl Code {
    /// 写给客人的那一格。
    pub const fn wire(self) -> u8 {
        match self {
            Code::Ok => WIRE_OK,
            Code::Denied => WIRE_DENIED,
            Code::Unjudged => WIRE_UNJUDGED,
        }
    }

    /// 放行了吗——调用点只该问这一句。
    pub const fn passed(self) -> bool {
        matches!(self, Code::Ok)
    }
}

/// 判一格：`facts` 是那几条边（[`Facts`] 的四问），`permit` 是那一格自己那一句话。
///
/// **照实记（这里原先还站着一层 trait，已折平）**：从前本文件有一枚 `Control`（四问 ＋
/// `has_face`）、一枚"还没有门牌"的假实现 `Blind`，以及一枚把 `Control` 四问逐个转发成
/// `judge` 要的四 trait 的 `Facts<'a, C>`。三样加起来只办一件事：让 `judge` 不认得
/// 树域那一份具体实现——而那个理由的**唯一消费者是已删的宿主靶**。折叠之后
/// **`Facts` 就是 `judge` 直接问的那四条边**（定义在 `judge.rs`），本文件只做"判 → 线上码"
/// 这一手。`Code::Blind` 随之退场：它只可能由 `has_face() == false` 产生，而那条路
/// （"手里还没门牌"）生产里由 `may` 在更早处短路。
pub fn verdict(facts: &impl Facts, who: TaskId, permit: Permit) -> Code {
    match judge(facts, who, permit) {
        Ruling::Allow => Code::Ok,
        Ruling::Deny => Code::Denied,
        Ruling::Unjudged => Code::Unjudged,
    }
}

// ── 本文件没有一行测试（用户裁定"protocol-case 没必要"）────────────────
//
// 原先那个 `#[cfg(test)] mod tests`（**6 条**）曾搬进宿主靶的 `judge` 靶，那台靶与那门判据
// 已一并删。那六条里唯靶里没有的是两处：三格码 ↔ 线上那一格的映射、以及"服务在但答不上来"
// 那一颗桩（`Mute`）——**今天一处判据都没有**。（第三处 `Blind` 对 `Under` / `Opens` 也答
// `Blind` 随 `Blind` 一起退场。）
