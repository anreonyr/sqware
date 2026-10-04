//! 一具架那一枚**铃** —— **通知那一层**：三拍（响 / 等 / 应），不碰页里的字节。
//!
//! 与 [`super::ring`] 的分工：那边是"字节过边界的那一具环"（有界、有游标、有四格账），
//! 这边是"有事"这一句。两端各自把两者拼起来：写端落完格响一下，读端读干了应一下。
//!
//! # 铃不是另一枚 Pie：它就是**页上那一位**
//! 一具架从前是"一枚页 ＋ 一枚铃"两件东西；把铃并进页之后（`env::abi::call` 的 `Ring`/
//! `Hush`/`Wait` 都认 [`Pole`](env::PieCall)），**一枚页就是一具完整的架** ——树上一格门牌
//! 挂得下的正是它。故这里的 [`Bell`] 只持**那一枚页的号**，三拍走页上那一位。
//!
//! # 两条要记住的事实
//! - **`wait` 不清那一位**：读端读干之后必须显式 [`Bell::hush`]——清必须与"取完"同一刻，
//!   否则醒来时那一位还亮着，下一趟 `wait` 当场就返 `true`（一圈空转）。
//! - **`Hush` 对页不碰本 hart 的中断闸门**（与孔上那一位同一条；门铃 Nole 那一支才
//!   `sie::set_sext`）。这正是"铃并进页"的一个好处：驱动那颗 hart 的闸门仍只由它自己那一手
//!   （`line.exhaust()`）重开。
//!
//! **一位（bool）而不是一列位**：写端每落一格响一次，而读端用的是"读干 → 应铃 → 复探 → 等"
//! 四拍（见 [`super::reader`]）——重复的响本就该并成一枚，多响的那几次答 `Busy`、写端当"正好"。

use env::{MailResult, PieToken, Wait};

/// 一具架那一枚铃（＝**那枚页上的一位**）。
pub(crate) struct Bell {
    pie: PieToken,
}

impl Bell {
    /// 拿那一枚页的号（架的持有者开页时就顺手有了；对端拿 `Rack::ship()` 交出的号重建）。
    pub(crate) fn from_token(page: PieToken) -> Self {
        Self {
            pie: page,
        }
    }

    /// 本端那一枚的号（要交给别人听时用）。
    pub(crate) fn token(&self) -> PieToken {
        self.pie
    }

    /// 响一下：置"有待取之事"并唤醒听者。**已响即 `Busy`，不是错**——写端不当它是失败。
    pub(crate) fn ring(&self) -> MailResult<()> {
        env::mail::ring(self.pie)
    }

    /// 等铃：`true` = 当场就绪（未挂起），`false` = 期限内没等到。**不清那一位**。
    pub(crate) fn wait(&self, within: Wait) -> MailResult<bool> {
        runtime::core::res::pie::HolePie::from_token(self.pie).wait(env::HoleDir::Pull, within)
    }

    /// 应一下：清掉"有待取之事"。**已经清着 ⇒ `Busy`**（读端把它当"正好"）。
    pub(crate) fn hush(&self) -> MailResult<()> {
        env::mail::hush(self.pie)
    }
}
