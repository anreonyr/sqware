//! desk —— **持树者侧那本账**：一位客人一格（谁 / 问 / 答）。
//!
//! 与 [`core`](super::core) 同一分工：本文件**不碰内核**（判据只有一条可机械检查的纪律——
//!
//! > `desk.rs` 里不出现 `runtime::`。
//!
//! ），探活是**注入的事实**（[`VestedBy`]，与 [`Operator`](super::core::Operator) 同款）：
//! 喂一个假闭包就能推理这本账，换载体不必重写。
//!
//! # 与 `board` 那本账的差别
//!
//! `board` 的客人账有两档：**听来的**（客人说了 `EVICT`，当场撤格）与**看出来的**
//! （那一枚答不出即剔）。这一本**只有看出来的那一档**——本正文里没有"客人退场"这件事：
//! 谁来问都答，问完就走，树不记账。
//!
//! 那为什么还要这本账？因为它记的不是寿命，是**坐标**：持树者醒来时手里只有"本表里的
//! 哪一枚有话"（`Pile::await_` 的返回），要答话就得知道**这一枚是谁的、答往哪走**。
//! `sweep` 因此仍在：剔的不是"走了的客人"，是**答不出的那一枚**——不剔，八格会被死客人
//! 占满，后来的连门都进不来。

use env::{PieToken, TaskId};

use protocol::system::operator::core::VestedBy;

// ── 这本账自己的失败域 ──────────────────────────────────────

/// `admit` 的两种不成——**每格一个不同的下一步**：**重放**不动账（接着办下一件事），
/// **满了**报一句（别静默丢一位客人）。
///
/// **照实记（这一格原来借的是别人的名字）**：`admit` 原先答 `protocol::system::operator::core::Fail`
/// 的 `NonEmpty`——那个名字在协议里说的是"**这块 Pane 非空**，要动它先清空"（`land` / `trim`
/// 的下一步），与"这位客人已经在账上"（重放，无事）**不是同一个下一步**。借名字的代价正是
/// 这一格：同一个码两种读法，读代码的人得先知道是谁发的。
///
/// 这本账的失败**一句都不上线**（只有持树者自己听得见），故它不必是协议那一份 `Fail`：那一份
/// 的每一格都要有线上码（`fail_codes!` 是**双射表**，加一格 = 加一个线上码）。名字也就不叫
/// `Fail`——它不是一个族的域，是一问的两种不成。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Admit {
    /// 这一位已经在账上（重放）：**不动账**——换掉就把原来那位的问话孔丢了。
    ///
    /// **照实记（这一趟交来的那一枚也不动，为什么不丢东西）**：重放时调用方手里那一枚
    /// `reply` 就是**原来那一枚**——它按 `(开者 == 这一位, 记号 == 树路)` 认出来
    /// （`server.rs` 的 `reply_of`），而一位客人这条路只 `seat` 一次 ⇒ 认出来的必然是同一枚。
    /// 故"不动账"在这里等于"原样"，不是"丢掉新交来的那条路"。
    Already,
    /// 备不下下一格（`try_reserve`）。
    Full,
}

// ── 一格 ────────────────────────────────────────────────────

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

// ── 账 ──────────────────────────────────────────────────────

/// 持树者侧的账：一叠格子，每格写着「谁 | 问在哪 | 答往哪」。
///
/// ```text
///   Admit   收一位客人（答话路到手）        —— 持树者侧：来客人了
///   Arm     记下它的问话孔在本表里的号      —— 先 arm 才 attach
///   Guest   由问话孔的号直达那一格          —— 醒来时唯一要问的一句
///   Sweep   剔走已经答不出的格子            —— 惰性，不是轮询
/// ```
pub struct Desk {
    /// **可增长**：不是定长数组——按需 `try_reserve`，备不下如实报 [`Admit::Full`]，不 `abort`。
    /// 条数是策略、容器要有界，那一格落在**分配**上，不落在常数上。
    ///
    /// **照实记（上限被撞满过三次，三次都是同一个病）**：上限原来是编译期常数，先是 `8`——按
    /// "当年那七位客人"定的，而客人分两档：**常驻的**（`router` / `uart` / `rtc` / `principal`，
    /// 树上那枚门牌活到会话结束）与**会死的**（`guest` / `sleeper` / `subject` / `echo`……），
    /// `sweep` 又是惰性的（主循环每轮末尾剔一次）⇒"死客人刚走、下一位就来了"那一瞬还占着格。
    /// 结盟那一刀带来**第五位常驻**（`coalition` 要按名字找 `/sys/principal`）与又多一位会死的
    /// （`member`）⇒ `8` 卡满，抬到 `12`；门禁那一刀又加了**一位负证客人 + 一位协调客人**
    /// ⇒ `12` 再卡满。症状每次一样：最后上树的 `echo` 的 `admit` 答 [`Admit::Full`] **而它自己
    /// 不知道**，它的问话孔没人管、第二次问话堵在单槽上，整台机器收不了场（读数：
    /// `echo: console=false`，随后没有 `system halted`；门禁那一刀的实测是 `examine` 0/3）。
    /// **第三次不再抬常数**（`b1d1ae1`）：改 `Vec` + `try_reserve`，顺手把 `server.rs` 里那个
    /// 按常数开的栈数组也收掉。
    guests: alloc::vec::Vec<Option<Guest>>,
    vested_by: VestedBy,
}

/// 立一本账（一位客人一格）：**注入的是协议那一侧"读内核事实"的那一枚**
/// （`call::vested_by`，`Reserve` 那一问）——账是实现的，判定是协议的。
pub fn desk() -> Desk {
    Desk::new(protocol::system::operator::call::vested_by)
}

impl Desk {
    /// 立一本账：**探活**跟着账走——它对每一格同值，故不必逐个作参数传。
    ///
    /// **不预分配**（`Vec::new()`）：这本账在服务起手就叫一次，而它今天只装几位客人——
    /// 让 `admit` 按需 `try_reserve` 那一格去报 `Full`，比这里先按一个猜的数占一片更诚实。
    pub fn new(vested_by: VestedBy) -> Desk {
        Desk {
            guests: alloc::vec::Vec::new(),
            vested_by,
        }
    }

    /// 收一位客人（答话路到手时叫）。返它的格子号。
    ///
    /// 两种不成见 [`Admit`]：**满**与**这一位已经在账上**（重放：一位客人只能占一格，重来的
    /// 那位不当场换掉原来那位——换掉就把它的问话孔丢了）。
    pub fn admit(&mut self, who: TaskId, reply: PieToken) -> Result<usize, Admit> {
        if self.guests.iter().flatten().any(|g| g.who == who) {
            return Err(Admit::Already);
        }
        // 先找空格；没有就**新开一格**——备不下如实报 `Full`（不 `abort`）。
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
                self.guests.try_reserve(1).map_err(|_| Admit::Full)?;
                self.guests.push(Some(Guest {
                    who,
                    ask: None,
                    reply,
                }));
                Ok(self.guests.len() - 1)
            }
        }
    }

    /// 记下"这位客人的问话孔是**本表里的哪一枚**"。返**成不成**。
    ///
    /// **两桩不成合成一格**（号不在账上 / 这一格已经挂着一枚）：调用方的下一步相同——这一格
    /// 这一轮不 arm（`server.rs` 的 `settle` 就是这么用的）。**先 `arm` 才 `attach`**：`arm`
    /// 之后的号才会进组，故 [`Desk::guest`] 在"被组唤醒的那一枚"上**永不为 `None`**。
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

    /// 摘掉这一格挂的问话孔（换孔时用）；本就没挂即无事。返**成不成**（同 [`Desk::arm`]：
    /// 号不在账上 = 不成，而调用方那一侧同样无事可做）。
    pub fn unarm(&mut self, slot: usize) -> bool {
        let Some(guest) = self.guests.get_mut(slot).and_then(Option::as_mut) else {
            return false;
        };
        guest.ask = None;
        true
    }

    /// 这枚号是哪位客人的（醒来时唯一要问的一句）。认不出返 `None`——**不是失败**
    /// （持树者那一枚提示孔也叫醒同一次等待，它的号自然不在这本账上）。
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

    /// 剔走**已经答不出**的客人，返剔了几格；幂等。
    ///
    /// 判据是注入的那一格（在这棵树里 = [`VestedBy`]：**客人答话路那一枚还答得出吗**）——
    /// 那一枚答 `None`（不在我表里，**或**它那扇门已经封印）就剔。
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
}
