//! Tole 域：`ToleCall::*` 转发（造一个组 / 挂 / 摘 / 等）。
//!
//! 与 `env/mail.rs` 的分工同款：本文件是**裸函数层 + 类型化句柄**——每个函数封一次
//! envcall，零业务逻辑（业务在协议层：谁把哪几枚孔挂到一处、等到之后怎么办）。
//!
//! `await` 的契约与 `unit::fall` / `unit::join` 同款：**挂起过一侧返回恒是预置值**
//! （`PieToken::NONE`），调用方按 deadline 循环、醒来自己按组快照复核。

use env::{EnvResult, HoleDir, PieToken, ToleCall, ToleCallRet};

use super::mail::{HolePie, NolePie};

// ── 裸函数层（envcall 转发，零业务逻辑）──

/// 造一个空组 → 组的句柄。
///
/// `shared` = 这枚组允不许多个使用者（**造的时候定、之后不可变**，见 `env::fid` 的
/// `ToleCall::Unseal`）：`false` = 独占组（授出即移交、复制不出来），`true` = 共享组
/// （可 `accord` 复制给多个任务；组键的唤醒是提示型——放行全链）。
pub fn unseal(shared: bool) -> EnvResult<PieToken> {
    let r = ToleCall::Unseal { shared }.call()?;
    match r {
        ToleCallRet::Unseal(t) => Ok(t),
        _ => unreachable!(),
    }
}

/// 把 `pie` 的一个方向挂进 `tole`（同成员幂等）。
pub fn attach(tole: PieToken, pie: PieToken, dir: HoleDir) -> EnvResult<()> {
    let r = ToleCall::Attach { tole, pie, dir }.call()?;
    match r {
        ToleCallRet::Attach(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 从 `tole` 摘掉一格；没挂过即无事。
pub fn detach(tole: PieToken, pie: PieToken, dir: HoleDir) -> EnvResult<()> {
    let r = ToleCall::Detach { tole, pie, dir }.call()?;
    match r {
        ToleCallRet::Detach(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 等到组里任意一格有事：`(哪一枚, 哪个方向)`；`millis` 三态同全树。
///
/// `PieToken::NONE` = 没等到（或挂起过——见文件头注的契约）。
pub fn await_(tole: PieToken, millis: usize) -> EnvResult<(PieToken, HoleDir)> {
    let r = ToleCall::Await { tole, millis }.call()?;
    match r {
        ToleCallRet::Await(pair) => Ok(pair),
        _ => unreachable!(),
    }
}

// ── 类型化句柄 ──

/// 能当**一格成员**的东西：孔与铃。
///
/// 与内核侧 `mail::tole::Mate` 是同一条边界：页不进组（没有"有事"这回事），组也不
/// 进组（没有位，判据会变成沿图的递归）。用一个 trait 而不是收 `PieToken`，是为了
/// 让"能挂什么"在编译期就说得清。
///
/// `token` 不在 [`AnyPie`](super::mail::AnyPie) 里（那一位是"表示层转换，与 ABI 无关"）；
/// 本 trait 的存在理由正是要那个号，故它自带一支。
pub trait Mate {
    /// 本成员在**我这张表**里的号。
    fn token(&self) -> PieToken;
}

impl Mate for HolePie {
    fn token(&self) -> PieToken {
        HolePie::token(self)
    }
}

impl Mate for NolePie {
    fn token(&self) -> PieToken {
        NolePie::token(self)
    }
}

/// 组的用户态句柄（与 [`HolePie`] 同款：只持一枚号，资源实体在内核）。
///
/// 方法集 = 这一个对象上能做的三件事（`unseal` 是**构造**，做不成 `&self` 方法）。
pub struct TolePie {
    token: PieToken,
}

impl TolePie {
    /// 造一个空组（种类见 [`crate::env::tole::unseal`]）。
    pub fn unseal(shared: bool) -> EnvResult<Self> {
        Ok(Self {
            token: crate::env::tole::unseal(shared)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的组）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 把一枚成员的一个方向挂进来。
    pub fn attach<M: Mate>(&self, mate: &M, dir: HoleDir) -> EnvResult<()> {
        crate::env::tole::attach(self.token, mate.token(), dir)
    }

    /// 摘掉一格。
    pub fn detach<M: Mate>(&self, mate: &M, dir: HoleDir) -> EnvResult<()> {
        crate::env::tole::detach(self.token, mate.token(), dir)
    }

    /// 等到任意一格有事。
    pub fn await_(&self, millis: usize) -> EnvResult<(PieToken, HoleDir)> {
        crate::env::tole::await_(self.token, millis)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}
