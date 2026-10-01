//! system::desk — **两枚域共用的一本账**：待客账（谁在跟我说话 / 它的问话孔 / 它的答话路）。
//!
//! **照实记（两本并成一本）**：板那一本与持树者那一本原是同一个 `Guest`、同一句"一位客人
//! 一格（谁 / 问 / 答）"逐字抄了两份，只是一本用定长数组、一本用 `Vec`；并的理由不是"少一个
//! 文件"——底下那几段把两处调用方各取所需的那几手一条条记着。
//!
//! **它为什么住这一格**：两枚消费者分居两枚域——板线程在编排域（`board/server.rs`）、持树者在
//! operator 域（`operator/server.rs`）——故它住**它们共同的那一格**。生命轴那一本（服务表）
//! 这一刀搬去了 `control/desk.rs`：那一本只有一枚域读。

use alloc::vec::Vec;

use env::{Mark, PieToken, TaskId};

// ── 客人账 ──────────────────────────────────────────────────
//
// **两本并成一本**（照实记）：板那一本（`board/desk.rs`）与持树者那一本（`operator/desk.rs`）
// 原是**同一个 `Guest`、同一句"一位客人一格（谁 / 问 / 答）"逐字抄了两份**，只是一本用
// `[Option<Guest>; CAP = 16]`、一本用 `Vec`。并的理由不是"少一个文件"：**板那一本的上限史
// 正是树那一本踩过的那一脚**——定长上限在树那一侧撞满过三次（`8` → `12` → 还是满，
// `b1d1ae1` 才改 `Vec`），而板那一侧还停在 `CAP = 16` 上，靠 `programs/src/system/inner.rs`
// 末尾一条 `const _` 断言挡着。**同一课两个人各学一遍**就是漏；并成一本之后它只有一处可记。
//
// 两本原本的差别**全部落在手上**，不落在容器上（并完之后两处调用方各取所需）：
// `evict`（听来的那一档：客人说了 `EVICT`）只有板用；`arm_pending` / `sweep_each` 两边都用。

/// 这本账的两种不成——**每格一个不同的下一步**：**重放**不动账（接着办下一件事），
/// **满了**报一句（别静默丢一位客人）。
///
/// **照实记（这一格原来借的是别人的名字）**：`admit` 原先答板那一份 `Fail` 的 `NonEmpty`
/// ——那个名字在协议里说的是"**这块 Pane 非空**，要动它先清空"，与"这位客人已经在账上"
/// （重放，无事）**不是同一个下一步**。借名字的代价正是这一格：同一个码两种读法。
///
/// 它也**不是**协议那一份 `Fail`：这本账的失败**一句都不上线**（只有招待客人的那一枚线程
/// 自己听得见），故不必进那张双射码表（`fail_codes!` 加一格 = 加一个线上码）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeskFail {
    /// 这一位已经在账上（重放）：**不动账**——换掉就把原来那位的问话孔丢了。
    Already,
    /// 备不下下一格（`try_reserve`）。
    Full,
}

/// 一位客人：**谁 / 问 / 答**。
///
/// `ask` 是 [`Option`]：客人"到了"（答话路到手）与"能问话了"（问话孔挂进来）是两步，
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

    /// 这一格挂上问话孔了没有（`armed` 才可能被组唤醒）。
    pub fn armed(&self) -> bool {
        self.ask.is_some()
    }
}

/// 客人账：一叠格子，每格写着「谁 | 问在哪 | 答往哪」。
///
/// ```text
///   Admit        收一位客人（答话路到手）      —— 来客人了
///   Evict        客人说了"我走了"，撤它那一格  —— 与 admit 成对；只有板用（听来的那一档）
///   Arm          记下它的问话孔在本表里的号    —— 先 arm 才 attach
///   Guest        由问话孔的号直达那一格        —— 醒来时唯一要问的一句
///   Sweep        剔走已经答不出的格子          —— 惰性，不是轮询（看出来的那一档）
/// ```
///
/// **可增长**：不是定长数组——按需 `try_reserve`，备不下如实报 [`DeskFail::Full`]，不 `abort`。
/// 条数是策略、容器要有界，那一格落在**分配**上，不落在常数上。
pub struct Desk {
    guests: Vec<Option<Guest>>,
}

impl Desk {
    /// 立一本账。**不预分配**（`Vec::new()`）：这本账起手时只装几位客人——让
    /// [`Desk::admit`] 按需 `try_reserve` 那一格去报 `Full`，比这里先按一个猜的数占一片更诚实。
    ///
    /// **探活那一手不在这里注入**（照实记）：从前它跟账走（`Desk { vested_by }`），
    /// 于是多出一个构造点、多一个类型别名、多一层"谁来接"；而它只有一个身体
    /// （[`vested_by`](protocol::communication::establish::vested_by)）——直接叫就是。
    pub const fn new() -> Desk {
        Desk { guests: Vec::new() }
    }

    /// 收一位客人（答话路到手时叫）。返它的格子号。
    ///
    /// 两种不成见 [`DeskFail`]：**满**与**这一位已经在账上**（后者是提示那条单槽路上的重放：
    /// 一位客人只能占一格，重来的那位要**报出来**，不能静默换掉原来那位——换掉就把它的
    /// 问话孔丢了）。
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

    /// 撤这一位客人的格子，返**撤掉的格子号**；不在账上 ⇒ `None`。
    ///
    /// 与 [`Desk::admit`] 成对：一位客人一格，进来一格、走了一格。**只动账**——把它的问话孔
    /// 从组里摘掉那一手不归它（组不在这一层）。
    ///
    /// 与 [`Desk::sweep`] 的分工：这一句撤的是**客人自己说了走**的那一格（听来的），
    /// `sweep` 剔的是**那一枚答不出**的那一格（看出来的）——故两句都在。
    pub fn evict(&mut self, who: TaskId) -> Option<usize> {
        let (slot, cell) = self
            .guests
            .iter_mut()
            .enumerate()
            .find(|(_, cell)| cell.as_ref().is_some_and(|g| g.who == who))?;
        *cell = None;
        Some(slot)
    }

    /// 记下"这位客人的问话孔是**本表里的哪一枚**"。返**成不成**。
    ///
    /// **两桩不成合成一格**（号不在账上 / 这一格已经挂着一枚）：调用方的下一步相同——这一格
    /// 这一轮不 arm。**先 `arm` 才 `attach`**：`arm` 之后的号才会进组，故 [`Desk::guest`]
    /// 在"被组唤醒的那一枚"上**永不为 `None`**。
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

    /// 摘掉这一格挂的问话孔（换孔或退场时用）；本就没挂即无事。返**成不成**（同
    /// [`Desk::arm`]：号不在账上 = 不成，而调用方那一侧同样无事可做）。
    ///
    /// **只被 [`Desk::arm_pending`] 用**（那两步要成对、任一步不成要回退），故不外露。
    fn unarm(&mut self, slot: usize) -> bool {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return false;
        };
        guest.ask = None;
        true
    }

    /// 这枚号是哪位客人的（醒来时唯一要问的一句）。认不出返 `None`——**不是失败**
    /// （提示孔那一路也叫醒同一次等待，它的号自然不在这本账上）。
    pub fn guest(&self, ask: PieToken) -> Option<&Guest> {
        self.guests.iter().flatten().find(|g| g.ask == Some(ask))
    }

    /// 还没挂上问话孔的那几格——**改得动**那一半：每格叫一次 `ask_of`，挂得上就 arm 并
    /// `attach`；返"**还有没有没补齐的**"。
    ///
    /// **`ask_of` 逐枚记号试**（`marks` = 本族认得的所有记号）：问话孔是按"**谁开的 ＋ 刻的
    /// 什么记号**"认的，而记号**不止一枚**——Operator 那一侧七枚操作面各刻一枚（见
    /// `protocol::service::operator::grant`）。故记号**不由本账写死**（本账不认识任何一族的面），
    /// 由调用方按自己那一族给；板那边只有一枚，传 `&[ASK_MARK]` 即可。
    ///
    /// **为什么这两手（`ask_of` / `attach`）要收进来**：调用方今天得先"抄一份'还没挂上的'"
    /// 到自己的栈上（`unarmed()` 借住这本账，而循环里要改它）；那一抄就是**一份按客人数的
    /// 分配**，或者**按一个常数开的数组**——后者正是这本账放弃的那件事（见上面的并本记）。
    /// 收进来之后调用方一格缓冲都不需要：遍历按**格子号**走，`arm` / `unarm` 都在本账里。
    ///
    /// **次序是硬的**：先 `arm` 再 `attach`；两步任一步不成 ⇒ `unarm` 回退（本就不挂即无事）
    /// 并报"还没补齐"。
    pub fn arm_pending(
        &mut self,
        marks: &[Mark],
        ask_of: impl Fn(TaskId, Mark) -> Option<PieToken>,
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
            // 逐枚记号试：**只有一枚**（板那一侧）时这一趟就一次扫表。
            let found = marks.iter().find_map(|mark| ask_of(who, *mark));
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

    /// 还没挂上问话孔的那几位——**只读**（诊断用；判据与 [`Desk::arm_pending`] 的"跳过"那一格同）。
    ///
    /// **为什么要有它**：`arm_pending` 把"是哪几位还没挂上"收在自己肚子里（那是它的判据），
    /// 而"**一直**挂不上"是另一件事——它得看得见，见 `operator::server::unarmed_report`。
    pub fn unarmed_each(&self, mut f: impl FnMut(TaskId)) {
        for guest in self.guests.iter().flatten() {
            if !guest.armed() {
                f(guest.who());
            }
        }
    }

    /// 账上还有几位。
    pub fn occupied(&self) -> usize {
        self.guests.iter().flatten().count()
    }

    /// 剔走**已经答不出**的客人，返剔了几格；幂等。
    ///
    /// 判据是注入的那一格（在这两棵树里 = [`VestedBy`]：**客人答话路那一枚还答得出吗**）——
    /// 那一枚答 `None`（不在我表里，**或**它那扇门已经封印）就剔。**看出来的**那一档；
    /// **听来的**那一档是 [`Desk::evict`]（客人自己说了走，账当场撤，不等它的门封印）。
    /// 两档都在，因为没说就走的那种也得有人收。
    pub fn sweep(&mut self) -> usize {
        self.sweep_each(|_| {})
    }

    /// 与 [`Desk::sweep`] **同判据**，但每剔一位叫一次 `f`（**趁它还认得出**），交给它的是**这一格
    /// 的三件事**：**谁**（推道要按名字认）、**它那条道**（`take_lane` 取走，取走即清）、
    /// **它的问话孔**（**读者结清／B-a**：那一枚还挂在组里 ⇒ 不摘就是一格永远醒不来的成员）。
    ///
    /// 板要用这个号去做第二件事：**推那一位的死亡道**。号只在这里拿得到——客人一旦退场，
    /// 它挂在板上的牌子随时会被摘掉，摘了就认不出"这一位叫什么"（道的记号是名字）。
    /// **道与号一起交出去**（照实记）：从前这一手只交号，板得拿号去**旁边那本同键的
    /// `Lanes`** 反查——那一本已并进 [`Guest`]，反查随它一起没了。
    ///
    /// **问话孔那一格是这一刀加的**（照实记）：被剔的客人从前只从**账**里消失，它挂进组的那
    /// 一枚仍在组里——每剔一位就多留一格再也醒不来的成员，而摘它需要那个号，只有这里拿得到。
    pub fn sweep_each(&mut self, mut f: impl FnMut(Gone)) -> usize {
        let mut gone = 0;
        for cell in self.guests.iter_mut() {
            if let Some(guest) = cell
                && protocol::communication::establish::vested_by(guest.reply).is_none()
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

/// **被剔走那一格的两样**（趁它还在账上）：谁 / 它的问话孔。
///
/// 两格各有各的下一步（摘组 / 都没有），故**一起交出去**而不是只交号——`Guest` 那本账剔完就
/// 把这一格清了，事后再反查就查不到"它挂的是哪一枚"。
///
/// **照实记（`lane` 那一格退场）**：它从前是这三位里的第一位（"它那条死亡道"）——道那一族
/// 上一刀在装配侧与监督侧都退了场（见 `control::supervise` 的 `Watch::new`），故板这一侧
/// 再也没有"往道里推一格"这件事可做。
pub struct Gone {
    /// 哪位客人。
    pub who: TaskId,
    /// 它交进来的问话孔（`None` = 还没挂上）。
    pub ask: Option<PieToken>,
}
