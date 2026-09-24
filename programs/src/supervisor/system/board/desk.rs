//! desk —— **板侧那本账**：一位客人一格（谁 / 问 / 答）。
//!
//! 与 [`core`](super::core) 同一分工：本文件**不碰内核**（判据只有一条可机械检查的纪律——
//!
//! > `desk.rs` 里不出现 `runtime::`。
//!
//! ），探活是**注入的事实**（[`VestedBy`]，与 [`Board`](super::core::Board) 同款）：喂一个假
//! 闭包就能推理这本账，换载体不必重写。
//!
//! 板线程醒来时手里只有**一枚孔在本表里的号**（`Tole::await_` 的返回），故这本账的读法
//! 就一句：[`Desk::guest`]——这枚号是谁的、答话往哪走。号是 [`Desk::arm`] 时记下的，而
//! **先 `arm` 才 `attach`** 是适配层的义务（`Tole` 那边只有挂过的孔才会被唤醒）⇒
//! "组里有一格认不出是谁"这种状态**不可表达**。

use env::{PieToken, TaskId};

use protocol::system::board::core::{Fail, VestedBy};

// ── 一格 ────────────────────────────────────────────────────

/// 一位客人：**谁 / 问 / 答**。
///
/// `ask` 是 [`Option`]：客人"到了"（答话路到手）与"能问话了"（问话孔挂进组）是两步，
/// 中间隔着装配者转授与客人自己交孔那两段路——故 `None` 是**一个状态**（还没挂上问话孔），
/// 不是错误。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guest {
    who: TaskId,
    ask: Option<PieToken>,
    reply: PieToken,
}

impl Guest {
    /// 哪位客人。
    pub fn who(&self) -> TaskId {
        self.who
    }

    /// 它的问话孔在**本表里的号**（`None` = 还没挂上）。
    pub fn ask(&self) -> Option<PieToken> {
        self.ask
    }

    /// 答话往哪儿走（本表里的号）。
    pub fn reply(&self) -> PieToken {
        self.reply
    }

    /// 这一格挂上问话孔了没有（`armed` 才可能被 `attach`）。
    pub fn armed(&self) -> bool {
        self.ask.is_some()
    }
}

// ── 账 ──────────────────────────────────────────────────────

/// 板侧的账：一叠格子，每格写着「谁 | 问在哪 | 答往哪」。
///
/// ```text
///   Admit   收一位客人（答话路到手）        —— 板线程侧：来客人了
///   Dismiss 客人说了"我走了"，撤它那一格     —— 与 admit 成对
///   Arm     记下它的问话孔在本表里的号      —— 先 arm 才 attach
///   Guest   由问话孔的号直达那一格          —— 醒来时唯一要问的一句
///   Sweep   剔走已经走了的格子              —— 惰性，不是轮询
/// ```
pub struct Desk {
    guests: [Option<Guest>; Desk::CAP],
    vested_by: VestedBy,
}

/// 立一本账（一位客人一格）：**注入的是协议那一侧"读内核事实"的那一枚**
/// （`call::vested_by`，`Reserve` 那一问）——账是实现的，判定是协议的。
pub const fn desk() -> Desk {
    Desk::new(protocol::system::board::call::vested_by)
}

impl Desk {
    /// 板侧最多几位客人。条数是策略，容器要有界。**这个数是板自己的账，管不着那个组**：
    /// 板线程的组是可增长的格子表（`mail/tole.rs` 的 `Vec<Cell>`），不替它定上限——
    /// "组能挂几格"是内存的事，不是本账的事。
    ///
    /// **照实记（8 → 16，跟着乙那一刀）**：这一格要 ≥ **同时**上板的客人数（验收那一景
    /// 上板的名册有 10 条：三枚内件 ＋ 三台驱动 ＋ 三个试客 ＋ `echo`），而 8 只剩一格余量
    /// ——加进持树者（乙）之后实测峰值 **6 → 7**。满了的后果是**静默**的：那条提示已经被
    /// 拉走，`admit` 答 `Full` 而调用方是 `let _ =` ⇒ 那一位从此没人监督、也没人报。
    /// 故抬到 **16**（与板自己那张牌子表的界 `Board::CAP` 同值）；**静默那一格仍挂着**
    /// （见 `4dfe335` 的照实记），今天够不着。
    pub const CAP: usize = 16;

    /// 立一本账：**探活**跟着账走——它对每一格同值，故不必逐个作参数传。
    pub const fn new(vested_by: VestedBy) -> Desk {
        Desk {
            guests: [None; Desk::CAP],
            vested_by,
        }
    }

    /// 收一位客人（答话路到手时叫）。返它的格子号。
    ///
    /// 两种不成：**满**（[`Fail::Full`]）与**这位已经在账上**（[`Fail::Taken`]）——后者是
    /// 提示那条单槽路上的重放：一位客人只能占一格，重来的那位要**报出来**，不能静默换掉
    /// 原来那位（换掉就把它的问话孔丢了）。
    pub fn admit(&mut self, who: TaskId, reply: PieToken) -> Result<usize, Fail> {
        if self.guests.iter().flatten().any(|g| g.who == who) {
            return Err(Fail::Taken);
        }
        let Some((slot, cell)) = self
            .guests
            .iter_mut()
            .enumerate()
            .find(|(_, cell)| cell.is_none())
        else {
            return Err(Fail::Full);
        };
        *cell = Some(Guest {
            who,
            ask: None,
            reply,
        });
        Ok(slot)
    }

    /// 撤这一位客人的格子，返**撤掉的格子号**；不在账上 ⇒ [`Fail::Unknown`]。
    ///
    /// 与 [`Desk::admit`] 成对：一位客人一格，进来一格、走了一格。**只动账**——把它的问话孔
    /// 从组里摘掉那一手不归它（`Tole` 不在这层，故那一步在程序层，见
    /// `super::server` 的退场那一支）。
    ///
    /// 与 [`Desk::sweep`] 的分工：这一句撤的是**客人自己说了走**的那一格，`sweep` 剔的是
    /// **那一枚答不出**（[`VestedBy`] 答 `None`）的那一格——一个是听来的，一个是看出来的，
    /// 故两句都在。
    pub fn evict(&mut self, who: TaskId) -> Result<usize, Fail> {
        let Some((slot, cell)) = self
            .guests
            .iter_mut()
            .enumerate()
            .find(|(_, cell)| cell.as_ref().is_some_and(|g| g.who == who))
        else {
            return Err(Fail::Unknown);
        };
        *cell = None;
        Ok(slot)
    }

    /// 记下"这位客人的问话孔是**本表里的哪一枚**"。
    ///
    /// **先 `arm` 才 `attach`**：`arm` 之后的号才会进组，故 [`Desk::guest`] 在"被组唤醒的
    /// 那一枚"上**永不为 `None`**。
    pub fn arm(&mut self, slot: usize, ask: PieToken) -> Result<(), Fail> {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return Err(Fail::Unknown);
        };
        if guest.ask.is_some() {
            return Err(Fail::Taken);
        }
        guest.ask = Some(ask);
        Ok(())
    }

    /// 摘掉这一格挂的问话孔（换孔或退场时用）；本就没挂即无事。
    pub fn unarm(&mut self, slot: usize) -> Result<(), Fail> {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return Err(Fail::Unknown);
        };
        guest.ask = None;
        Ok(())
    }

    /// 这枚号是哪位客人的（醒来时唯一要问的一句）。认不出返 `None`——**不是失败**
    /// （提示孔那一路也叫醒同一次等待，它的号自然不在这本账上）。
    pub fn guest(&self, ask: PieToken) -> Option<&Guest> {
        self.guests.iter().flatten().find(|g| g.ask == Some(ask))
    }

    /// 还没挂上问话孔的那几格：`(格子号, 谁)`——适配层照它去认客人自己交来的那一枚。
    pub fn unarmed(&self) -> impl Iterator<Item = (usize, TaskId)> + '_ {
        self.guests
            .iter()
            .enumerate()
            .filter_map(|(slot, cell)| cell.as_ref().filter(|g| !g.armed()).map(|g| (slot, g.who)))
    }

    /// 账上还有几位。
    pub fn occupied(&self) -> usize {
        self.guests.iter().flatten().count()
    }

    /// 剔走**已经走了**的客人，返剔了几格；幂等。
    ///
    /// 判据是注入的那一格（在这棵树里 = [`VestedBy`]：**客人答话路那一枚还答得出吗**），故
    /// **这一句**认的是**看出来的**那一档：那一枚答 `None`（不在我表里，**或它那扇门已经
    /// 封印**——见 `core::VestedBy`）就剔。**听来的**那一档是 [`Desk::evict`]——客人自己说了
    /// 走，账当场撤（不等它的门封印）。两档都在，因为没说就走的那种也得有人收。
    pub fn sweep(&mut self) -> usize {
        let vested_by = self.vested_by;
        let mut gone = 0;
        for cell in self.guests.iter_mut() {
            if let Some(guest) = cell
                && vested_by(guest.reply).is_none()
            {
                *cell = None;
                gone += 1;
            }
        }
        gone
    }

    /// 与 [`Self::sweep`] **同判据**，但把剔掉的那几位的号写进 `out`（返剔了几格）。
    ///
    /// 板要用这个号去做第二件事：**推那一位的死亡道**。号只在这里拿得到——客人一旦退场，
    /// 它挂在板上的牌子随时会被摘掉，摘了就认不出"这一位叫什么"（道的记号是名字）。
    pub fn sweep_who(&mut self, out: &mut [TaskId]) -> usize {
        let vested_by = self.vested_by;
        let mut gone = 0;
        for cell in self.guests.iter_mut() {
            if let Some(guest) = cell
                && vested_by(guest.reply).is_none()
            {
                if let Some(slot) = out.get_mut(gone) {
                    *slot = guest.who();
                }
                *cell = None;
                gone += 1;
            }
        }
        gone
    }
}
