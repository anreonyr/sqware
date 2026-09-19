//! Tole（组）：把几枚**可等地**（孔的一个方向 / 一枚铃）挂到一处，等其中**任意一格**有事。
//!
//! 与 `core/bell.rs` 同一分工：门铃是"一枚无载荷信号怎么用"，这里是"多路等待怎么用"
//! ——薄封装 + 让调用的形状像一句话；envcall 转发在 `env/tole.rs`。
//!
//! 为什么要有这一层：`await` 的返回是"哪一枚"（一枚号 + 一个方向），而**挂起过**的
//! 那一次读回来的是预置值 `PieToken::NONE`（内核没有第二次执行机会）。把这条契约翻成
//! `Option`（`None` = 这一轮没等到），调用方就不必自己认哨兵——但也**必须**按 deadline
//! 循环，否则 `None` 会被误当成"永远没有"。

use env::{EnvResult, HoleDir, PieToken};

use crate::env::tole::{Mate, TolePie};

/// 一个组的使用面。
pub struct Tole {
    pie: TolePie,
}

impl Tole {
    /// 造一个空组。
    ///
    /// `shared` = 允不许多个使用者（造的时候定、之后不可变）：`false` = 独占组（授出即
    /// 移交、复制不出来），`true` = 共享组（可交给多个任务各持一枚；组键的唤醒是**提示
    /// 型**——放行全链，人人醒来自己按组复核）。
    pub fn unseal(shared: bool) -> EnvResult<Tole> {
        Ok(Tole {
            pie: TolePie::unseal(shared)?,
        })
    }

    /// 收下一枚已经在对端的组（`Pier` 递过来的 token）。
    pub fn new(pie: TolePie) -> Tole {
        Tole { pie }
    }

    /// 把一枚成员的一个方向挂进来（同成员幂等）。
    pub fn hang<M: Mate>(&self, mate: &M, dir: HoleDir) -> EnvResult<()> {
        self.pie.hang(mate, dir)
    }

    /// 摘掉一格；没挂过即无事。
    pub fn unhang<M: Mate>(&self, mate: &M, dir: HoleDir) -> EnvResult<()> {
        self.pie.unhang(mate, dir)
    }

    /// 等到任意一格有事：`Some((哪一枚, 哪个方向))`；`None` = 这一轮没等到
    /// （挂起过，或期限到）——**继续等就再叫一次**，别把 `None` 当成终局。
    pub fn await_(&self, millis: usize) -> EnvResult<Option<(PieToken, HoleDir)>> {
        let (token, dir) = self.pie.await_(millis)?;
        Ok((token != PieToken::NONE).then_some((token, dir)))
    }

    pub fn token(&self) -> PieToken {
        self.pie.token()
    }
}
