//! rtc::core::slot — **那一格**：一台设备一个闹钟槽、两个原语（约 / 到点）。
//!
//! 它是**核的内件**：对外只经 [`super::Host`]（会话核持有它，一问/一投递都从它那里过），
//! 故"一台设备一个闹钟寄存器"这件事不由调用方维护，而是由 [`super::Host`] 手里那一格承载。
//!
//! 本文件**不碰设备**：不出现 `runtime::`、不出现 `View`——唯一的设备侧动作（读表、武装闹钟、
//! 清那一格）由适配层紧随原语之后做（它要动硬件，见 `src/driver/rtc/adapt/`），故核心只记
//! "约了哪个时刻、往哪回"，可独立推理。这与 `protocol::driver::line::core` 同一条分工
//! （那边是"账只记账，接线那几手由适配层做"）。
//!
//! **一台设备只有一个闹钟寄存器**——这一条由类型承载：那一格只有两个变体，"两位客人一个
//! 闹钟"写不出来。第二位客人来的时候格子非空 ⇒ 答 `Taken`，这是**决策 4（一位客人）的
//! 落法**，不是一条运行期策略。

use super::Fail;
use env::PieToken;

/// 那一格：空着，或者武装着（到点时刻 + **往哪回**）。
///
/// `back` = **客人自己开的那一枚孔**在本端表里的号。三件事因此都落在这一格里：
/// "约了哪个时刻"（`at`）、"到点往哪说"（`back`）、"那位客人还在不在"——最后那一件由
/// **那一枚孔自己答**（开者退场 ⇒ 它开的资源一起封印，见 `kernel` 的 `gate::cull`），
/// 故这一格没有名字、没有任务号、没有泊位：**没有"客人"这个类型**。
enum Cell {
    Idle,
    Armed { at: u64, back: PieToken },
}

/// 那一格（一台设备一格）。
pub struct Slot {
    cell: Cell,
}

impl Slot {
    /// 起一格空的。
    pub const fn new() -> Slot {
        Slot { cell: Cell::Idle }
    }

    /// **约**：占住那一格。
    ///
    /// 格非空 ⇒ `Taken`；`at` 已经过去 ⇒ `Past`。**成与不成都不动已有一格的内容**——
    /// 被拒的那一趟（`Taken` / `Past`）绝不能把别人约好的时刻改掉或撤掉。
    ///
    /// **`Past` 是本面的策略，不是设备的事实**：设备对过去的时刻是**当场就报**（低半格那次写
    /// 会立刻比较一次，见 `rtc.rs` 头注）；本面选择当场答 `Past`，因为"你约的时刻已经过去了"
    /// 是客人算错了，替他兑现那一声既不是他要的、也说不清是写给谁看的。故判据在这里，
    /// 而 `now` 由适配层从设备读出来交给它（核心不碰设备）。
    pub fn arm(&mut self, at: u64, back: PieToken, now: u64) -> Result<(), Fail> {
        if matches!(self.cell, Cell::Armed { .. }) {
            return Err(Fail::Taken);
        }
        if at <= now {
            return Err(Fail::Past);
        }
        self.cell = Cell::Armed { at, back };
        Ok(())
    }

    /// **到点**：把那一格取出来（**格因此回空**）、交出"往哪回"；没到点 / 空着 ⇒ `None`。
    ///
    /// **取走就是兑现**：往外推那一手（以及推不出去 ⇒ 那位客人没了）由适配层做，故这里
    /// 不另立"收回那一格"的动作——拒了的那一趟由 [`Slot::arm`] 自己保证没占上。
    pub fn fire(&mut self, now: u64) -> Option<PieToken> {
        match self.cell {
            Cell::Armed { at, back } if now >= at => {
                self.cell = Cell::Idle;
                Some(back)
            }
            _ => None,
        }
    }
}
