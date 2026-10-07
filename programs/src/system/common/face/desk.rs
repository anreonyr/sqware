//! 待客账（谁在跟我说话 / 它的问话孔 / 它的答话路）。

use alloc::vec::Vec;

use env::{PieToken, TaskId};

/// 这本账的两种不成——**每格一个不同的下一步**：**重放**不动账（接着办下一件事）
/// **满了**报一句（别静默丢一位客人）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeskFail {
    /// 这一位已经在账上（重放）：**不动账**——换掉就把原来那位的问话孔丢了
    Already,
    /// 备不下下一格（`try_reserve`）
    Full,
}

/// 一位客人：**谁 / 问 / 答**
/// `ask` 是 Option：客人"到了"（答话路到手）与"能问话了"（问话孔挂进来）是两步
/// 中间隔着装配者转授与客人自己交孔那两段路——故 `None` 是**一个状态**（还没挂上问话孔）
/// 不是错误
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guest {
    who: TaskId,
    ask: Option<PieToken>,
    reply: PieToken,
}

impl Guest {
    /// 哪位客人
    pub fn who(&self) -> TaskId {
        self.who
    }

    /// 它的问话孔在**本表里的号**（`None` = 还没挂上）
    pub fn ask(&self) -> Option<PieToken> {
        self.ask
    }

    /// 答话往哪儿走（本表里的号）
    pub fn reply(&self) -> PieToken {
        self.reply
    }

    pub fn armed(&self) -> bool {
        self.ask.is_some()
    }
}

/// 客人账：一叠格子，每格写着「谁 | 问在哪 | 答往哪」
/// **可增长**：不是定长数组——按需 `try_reserve`，备不下如实报 DeskFail::Full，不 `abort`
/// 条数是策略、容器要有界，那一格落在**分配**上，不落在常数上
pub struct Desk {
    guests: Vec<Option<Guest>>,
}

impl Desk {
    /// 立一本账。**不预分配**（`Vec::new()`）：这本账起手时只装几位客人——让
    pub const fn new() -> Desk {
        Desk { guests: Vec::new() }
    }

    /// 收一位客人（答话路到手时叫）。返它的格子号
    /// 两种不成见 DeskFail：**满**与**这一位已经在账上**（后者是提示那条单槽路上的重放
    /// 一位客人只能占一格，重来的那位要**报出来**，不能静默换掉原来那位——换掉就把它的
    /// 问话孔丢了）
    pub fn admit(&mut self, who: TaskId, reply: PieToken) -> Result<usize, DeskFail> {
        if self.guests.iter().flatten().any(|g| g.who == who) {
            return Err(DeskFail::Already);
        }
        // 先找空格；没有就**新开一格**——备不下如实报 `Full`。
        match self.guests.iter().position(|cell| cell.is_none()) {
            Some(slot) => {
                self.guests[slot] = Some(Guest {
                    who,
                    ask: None,
                    reply,
                });
                Ok(slot)
            }
            None => {
                self.guests.try_reserve(1).map_err(|_| DeskFail::Full)?;
                self.guests.push(Some(Guest {
                    who,
                    ask: None,
                    reply,
                }));
                Ok(self.guests.len() - 1)
            }
        }
    }

    /// 撤这一位客人的格子，返**撤掉的格子号**；不在账上 ⇒ `None`
    /// 与 Desk::admit 成对：一位客人一格，进来一格、走了一格。**只动账**——把它的问话孔
    /// 从组里摘掉那一手不归它（组不在这一层）
    /// 与 Desk::sweep 的分工：这一句撤的是**客人自己说了走**的那一格（听来的）
    /// `sweep` 剔的是**那一枚答不出**的那一格（看出来的）——故两句都在
    pub fn evict(&mut self, who: TaskId) -> Option<usize> {
        let (slot, cell) = self
            .guests
            .iter_mut()
            .enumerate()
            .find(|(_, cell)| cell.as_ref().is_some_and(|g| g.who == who))?;
        *cell = None;
        Some(slot)
    }

    /// 记下"这位客人的问话孔是**本表里的哪一枚**"。返**成不成**
    pub fn arm(&mut self, slot: usize, ask: PieToken) -> bool {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return false;
        };
        if guest.ask.is_some() {
            return false;
        }
        guest.ask = Some(ask);
        true
    }

    fn unarm(&mut self, slot: usize) -> bool {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return false;
        };
        guest.ask = None;
        true
    }

    /// 这枚号是哪位客人的（醒来时唯一要问的一句）。认不出返 `None`——**不是失败**
    /// （提示孔那一路也叫醒同一次等待，它的号自然不在这本账上）
    pub fn guest(&self, ask: PieToken) -> Option<&Guest> {
        self.guests.iter().flatten().find(|g| g.ask == Some(ask))
    }

    /// 还没挂上问话孔的那几格——**改得动**那一半：每格叫一次 `ask_of`，挂得上就 arm 并
    /// `attach`；返"**还有没有没补齐的**"
    /// **`ask_of` 逐枚记号试**（`marks` = 本族认得的所有记号）：问话孔是按"**谁开的 ＋ 刻的
    /// 什么记号**"认的，而记号**不止一枚**——Operator 那一侧每一位操作面各刻一枚（见
    /// 由调用方按自己那一族给；板那边只有一枚，传 `&[ASK_MARK]` 即可
    /// 到自己的栈上（`unarmed()` 借住这本账，而循环里要改它）；那一抄就是**一份按客人数的
    /// 分配**，或者**按一个常数开的数组**——后者正是这本账放弃的那件事（见上面的并本记）
    /// 收进来之后调用方一格缓冲都不需要：遍历按**格子号**走，`arm` / `unarm` 都在本账里
    /// **次序是硬的**：先 `arm` 再 `attach`；两步任一步不成 ⇒ `unarm` 回退（本就不挂即无事）
    /// 并报"还没补齐"
    pub fn arm_pending(
        &mut self,
        find: impl Fn(TaskId) -> Option<PieToken>,
        mut attach: impl FnMut(PieToken) -> bool,
    ) -> bool {
        let mut pending = false;
        for slot in 0..self.guests.len() {
            let Some(who) = self.guests[slot].as_ref().map(|g| g.who) else {
                continue;
            };
            if self.guests[slot].as_ref().is_some_and(|g| g.armed()) {
                continue;
            }
            let found = find(who);
            match found {
                Some(ask) => {
                    let hung = self.arm(slot, ask) && attach(ask);
                    if !hung {
                        let _ = self.unarm(slot);
                        pending = true;
                    }
                }
                None => pending = true,
            }
        }
        pending
    }

    /// 还没挂上问话孔的那几位——**只读**
    /// **为什么要有它**：`arm_pending` 把"是哪几位还没挂上"收在自己肚子里
    /// 而"**一直**挂不上"是另一件事——它得看得见，见 operator::serve::unarmed_report
    pub fn unarmed_each(&self, mut f: impl FnMut(TaskId)) {
        for guest in self.guests.iter().flatten() {
            if !guest.armed() {
                f(guest.who());
            }
        }
    }

    /// 账上还有几位
    pub fn occupied(&self) -> usize {
        self.guests.iter().flatten().count()
    }

    /// 剔走**已经答不出**的客人，返剔了几格；幂等
    /// 判据是注入的那一格（在这两棵树里 = VestedBy：**客人答话路那一枚还答得出吗**）——
    /// 那一枚答 `None`（不在我表里，**或**它那扇门已经封印）就剔。**看出来的**那一档
    /// **听来的**那一档是 Desk::evict（客人自己说了走，账当场撤，不等它的门封印）
    /// 两档都在，因为没说就走的那种也得有人收
    pub fn sweep_each(&mut self, mut f: impl FnMut(Gone)) -> usize {
        let mut gone = 0;
        for cell in self.guests.iter_mut() {
            if let Some(guest) = cell
                && ipc::session::establish::vested_by(guest.reply).is_none()
            {
                f(Gone {
                    who: guest.who(),
                    ask: guest.ask,
                });
                *cell = None;
                gone += 1;
            }
        }
        gone
    }
}

/// **被剔走那一格的两样**（趁它还在账上）：谁 / 它的问话孔
/// 两格各有各的下一步（摘组 / 都没有），故**一起交出去**而不是只交号——`Guest` 那本账剔完就
pub struct Gone {
    /// 哪位客人
    pub who: TaskId,
    /// 它交进来的问话孔（`None` = 还没挂上）
    pub ask: Option<PieToken>,
}
