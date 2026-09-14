//! port — Hole 的通讯协议：授出（`ship`）、坐标（`To`）、一次往返（`Port`）、报文对（`Duet`）。
//!
//! 一个类型回答一个问题，一个也不多：
//!
//! ```text
//! Access / Policy   对端能做什么 / 这一枚能怎么流动（四位分两族，混族不可表达）
//! ship              授出：子集由两族拼出，空集本地拒（不发 envcall）
//! To                对端坐标：号（`who`）与它在**那张表里**的句柄（`token`）——两半成对
//! Duet              报文对的形状：请求与回复成对绑定，布局归协议
//! Port              一枚 Hole 门闩上的一次往返：填回信地址 / push / 校来源 / 有界等 / 解码
//! ```
//!
//! **本层不是内核对象**：槽在内核 `mail`、权柄判定与级联在内核 `gate`、报文布局在
//! `crates/protocol`，这里只有"怎么用"。
//!
//! **`Port` 是 `Channel` 的替代**（旧类型已删）。它多出的三件事各对应一条教训：
//! 坐标两半成对（旧类型只存对端那半）、收尾只 `release`（级联已含对端那枚副本）、
//! 协议拿不到句柄（旧类型 `mine()` 交出整枚门闩）。
//!
//! 判据与裁决账见 `docs/port.md`。

use core::ops::BitOr;

use env::{EnvError, EnvResult, Permission, PieToken, TaskId, make_err};

use crate::env::mail::{self, AnyPie, HolePie};

/// D1 负码：无权 / 协议错（与 `crates/protocol` 各协议的负码同表）。
fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 读写族：对端对这份资源**能做什么**。
///
/// 与 [`Policy`] 分成两个类型是刻意的：合成一个 `Permission` 时，调用方可以把
/// `CAGE` 写进"读写"该在的位置，而两族相乘的十六格里有一格是空集、一格只有形态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Access(Permission);

impl Access {
    /// 什么都没有：两族都取 `NONE` 才是空集（那种授予 `ship` 就地拒）。
    pub const NONE: Access = Access(Permission::empty());
    /// 观察 / 接收 / 重读。
    pub const READ: Access = Access(Permission::READ);
    /// 修改 / 投递 / 写入。
    pub const WRITE: Access = Access(Permission::WRITE);

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
    }
}

impl BitOr for Access {
    type Output = Access;

    fn bitor(self, rhs: Access) -> Access {
        Access(self.0 | rhs.0)
    }
}

/// 传递族：这一枚**能怎么流动**。
///
/// 四个取值读作四件事：
///
/// ```text
/// NONE         分  双方各持一份，对端不能再授出
/// VEST         借  双方各持一份，对端可以再授出
/// CAGE         让  我失去这一枚，对端不能再授出
/// VEST | CAGE  给  我失去这一枚，对端可以再授出（粘性：它再授出的必带 CAGE）
/// ```
///
/// **`READ` 只可转移、不可复制**：授出 `READ` 不带 `CAGE` ⇒ 源枚仍可用 ⇒ 两个读者。
/// 单读者因此不是内核机制，是位表的推论。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Policy(Permission);

impl Policy {
    /// 不能再授出（源枚不动）。
    pub const NONE: Policy = Policy(Permission::empty());
    /// 对端可以再授出。
    pub const VEST: Policy = Policy(Permission::VEST);
    /// 一次交出：源枚在交出期间不可用。
    pub const CAGE: Policy = Policy(Permission::CAGE);

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
    }
}

impl BitOr for Policy {
    type Output = Policy;

    fn bitor(self, rhs: Policy) -> Policy {
        Policy(self.0 | rhs.0)
    }
}

/// 对端坐标：**两半成对**——对端是谁（`who`），以及那枚副本在**对端表里**的句柄
/// （`token`）。
///
/// 号只在那一张表里有意义，故两半不能分家：字段私有、只有 [`ship`] 能造。旧
/// `Channel` 的病根正是只存了对端那半（`at_peer`）。
#[derive(Clone, Copy, Debug)]
pub struct To {
    who: TaskId,
    token: PieToken,
}

impl To {
    fn new(who: TaskId, token: PieToken) -> To {
        To { who, token }
    }

    /// 对端是谁。用于校验"这条回复真是它推的"。
    pub const fn who(self) -> TaskId {
        self.who
    }

    /// 那枚副本**在对端表里**的句柄——写进报文，对端按它 push。
    pub const fn token(self) -> PieToken {
        self.token
    }
}

/// 授出：把 `pie` 的一个子集授给 `who`，返对端坐标。
///
/// **这就是"授出"的全部机制**：子集由两族拼出，混族写不出来；空集本地拒（不发
/// envcall）。调用点从此写不出裸子集——一份子集写错就是一份多余授权，而
/// `Access`/`Policy` 让"该授什么"在签名上就说清楚了。
///
/// 泛型于 [`AnyPie`]：三种资源都有授出点（回信孔是 `HolePie`、设备门闩是
/// `PolePie`、建域权是 `NolePie`），权柄操作本就与资源种类无关。
///
/// # Errors
/// - `Denied` — 空集（本地拒）/ 源枚不持 `VEST` / 子集越界 / 已被关住 / 对端不存在
/// - `Dead`   — 源枚的资源已封印
/// - `OoM`    — 对端表备不出容量
pub fn ship<P: AnyPie>(pie: &P, who: TaskId, access: Access, policy: Policy) -> EnvResult<To> {
    let subset = access.bits() | policy.bits();
    if subset.is_empty() {
        return Err(denied());
    }
    let token = pie.accord(who, subset)?;
    Ok(To::new(who, token))
}

/// 报文对的形状：一条请求、一条回复，**成对绑定**。
///
/// 用 trait 而非闭包：闭包版可以把 A 协议的 `encode` 与 B 协议的 `decode` 一起递
/// 进去，编译器不响；`type Req` / `type Rep` 配错不可表达。
pub trait Duet {
    /// 请求载荷。
    type Req;
    /// 回复载荷。
    type Rep;
    /// 报文容器（`[u8; REQ]` 那个数组）。
    ///
    /// 为什么容器由协议自己给：稳定 Rust 里 `[u8; W::REQ]` 是泛型常量表达式
    /// （要 `generic_const_exprs`），用不了 ⇒「报文多大」这份知识只能留在实现侧。
    type Wire: AsRef<[u8]> + AsMut<[u8]>;

    /// 请求报文的字节数（**协议自己的尺寸**：孔不替协议记长度）。
    const REQ: usize;
    /// 回复报文的字节数。
    const REP: usize;

    /// 一个空的报文容器。请求与回复**共用一块**：报文推出去之后即可复用——`push`
    /// 在锁外就把字节搬进了内核那份 staging。
    fn wire() -> Self::Wire;

    /// 布局：把请求写进 `out`，并把**回信地址**写在该协议自己的偏移上。
    ///
    /// `at` = 本端回信孔在**对端表里**的句柄（[`To::token`]）——布局归协议：
    /// 地址写哪、写成几个字节，它说了算。
    fn encode(req: &Self::Req, at: PieToken, out: &mut [u8]);

    /// 解码回复。报文不合法 ⇒ `Denied`。
    fn decode(buf: &[u8]) -> EnvResult<Self::Rep>;
}

/// 一条会话：入口门闩（借入）+ 回信孔（自有）+ 对端坐标。
///
/// `entry` 的所有权**不在本类型**——只借入，`close` 不碰它：今天 `Service` 释放
/// entry、`Console` 不释放，那是**策略**，不进结构。
pub struct Port {
    to: To,
    entry: HolePie,
    reply: HolePie,
}

impl Port {
    /// 开一条会话：`entry` = 对端入口门闩在**我这一份**表里的句柄。
    ///
    /// 做三件事：问出对端是谁（`Reserve(entry).owner`——`vestor` 会被转发改写，
    /// `owner` 不会）、自建回信孔、把它的 `WRITE` 授给对端。回信孔只要 `WRITE`：
    /// 对端往它推，读的一侧是我。
    ///
    /// # Errors
    /// - `Denied` — 入口门闩不在表里 / 无开辟者 / 授不出去
    pub fn open(entry: &HolePie) -> EnvResult<Port> {
        let who = mail::reserve(PieToken::new(entry.token()))?.1;
        if who.get() == 0 {
            return Err(denied());
        }
        let reply = HolePie::unseal()?;
        let to = ship(&reply, who, Access::WRITE, Policy::NONE)?;
        Ok(Port {
            to,
            entry: HolePie::from_token(entry.token()),
            reply,
        })
    }

    /// 一次往返：填回信地址 → `push`（满则等＝背压）→ 有界等 → **校来源** → 解码。
    ///
    /// `within` = 等回复的毫秒上界（`usize::MAX` = 永久）。有上界才能把"回复被丢"
    /// 暴露成 `Busy`，而不是永久挂起；而"用户还在打字"这一类往返要的正是不设上界。
    ///
    /// **来源对不上 ⇒ `Denied`，该会话应弃用**：迟到的真回复仍可能落槽、污染下一次
    /// `call`（与 [`HolePie::pull_timeout`] 的既有契约同款）。
    pub fn call<W: Duet>(&self, req: &W::Req, within: usize) -> EnvResult<W::Rep> {
        let mut wire = W::wire();
        W::encode(req, self.to.token(), wire.as_mut());
        self.entry
            .push(wire.as_ref().get(..W::REQ).ok_or_else(denied)?)?;
        let field = wire.as_mut().get_mut(..W::REP).ok_or_else(denied)?;
        let (len, from) = self.reply.pull_timeout_from(field, within)?;
        if from != self.to.who() {
            return Err(denied());
        }
        W::decode(wire.as_ref().get(..len).ok_or_else(denied)?)
    }

    /// 关：只放下回信孔（**级联已含对端那枚副本**，不必再 `revoke`——那是旧
    /// `Channel` 的冗余动作，它一旦失败还会短路后续清理）。
    pub fn close(self) -> EnvResult<()> {
        self.reply.release()
    }
}
