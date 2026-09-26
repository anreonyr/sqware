//! board 的核心 —— **公示板、牌子、失败域，与那四个动作**。
//!
//! 本文件**不碰内核**——这条纪律现在由 **crate 边界**管着（`contract` 全树不碰内核那一层，
//! 见本 crate 的头注），故这里不再复述。
//!
//! "谁授的这枚入口""这枚入口还答得出吗""放下它"全是**注入的事实与动作**（[`VestedBy`] /
//! [`Unship`]），故一块板的规矩喂两个假闭包就能推理，换载体不必重写。

use env::{Name, PieToken, TaskId};

use crate::session::{Claim, Seat};

// ── 结构 ────────────────────────────────────────────────────

/// 板上的一枚牌子：**叫什么、在哪、谁挂的**。
///
/// 入口是 [`Option`] 而不是哨兵值：**"没有实例"是一个状态**（从未登记 / 已注销 / 已死
/// 三者同一个状态），故 `Unknown` 只有一种语义。
///
/// `owner` 是挂牌人：它在**有实例**时才有意义，而实例一死 `sweep_at` 就把牌子扫空
/// （连 `owner` 一起）——**故它不可能过期**，不需要第二套"owner 还在吗"的规则。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sign {
    name: Name,
    entry: Option<PieToken>,
    owner: Option<TaskId>,
}

impl Sign {
    /// 空牌子：板子的初值。
    pub const VACANT: Sign = Sign {
        name: Name::EMPTY,
        entry: None,
        owner: None,
    };

    /// 这一枚叫什么。
    pub fn name(&self) -> Name {
        self.name
    }

    /// 挂在哪（`None` = 没人挂 / 挂的人已经不在）。
    pub fn entry(&self) -> Option<PieToken> {
        self.entry
    }

    /// 谁挂的（`None` = 同上）。
    pub fn owner(&self) -> Option<TaskId> {
        self.owner
    }

    /// 牌子立着——有实例，故这一枚此刻答得出"在哪"。
    pub fn standing(&self) -> bool {
        self.entry.is_some()
    }

    /// 板上有没有这一枚的名字（诊断：牌子存在，哪怕此刻是空的）。
    pub fn named(&self) -> bool {
        !self.name.is_empty()
    }

    fn lift(&mut self) {
        self.entry = None;
        self.owner = None;
    }
}

/// 挂一枚牌子上板时，坏在哪一步。
///
/// 四个变体各对应**一个不同的下一步**：换个名字 / 摘掉旧牌 / 找持板者要入口 / 扩容。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 板上没这一枚：查不到（[`Board::lookup`]）/ 那一位已经不在了
    /// （[`Board::unregister`]）。**"板满"不是这一格**——那是 [`Fail::Full`]。
    Unknown,
    /// 这一枚已经有人挂着了（摘牌或换名，别抢）。
    Taken,
    /// 这枚入口不是你亲手交给持板者的。
    Denied,
    /// 板挂了（条数是策略，容器有界——与 `Service` 表、`Quay` 同款）。
    Full,
}

/// **定义在 `system::core`**（照实记：两个面原是各写一遍的同名同形别名；共用的客人账
/// 要的是**一个**类型 ⇒ 收成了一处）。
pub use crate::system::core::VestedBy;

/// **放下**：把自己那一份入口自释。牌子被换掉或扫空时用它，否则那枚门闩漏在板上。
pub type Unship = fn(PieToken) -> Result<(), ()>;

// ── 板 ──────────────────────────────────────────────────────

/// 一块公示板：一叠牌子，每枚写着「叫什么 | 在哪」。
///
/// 三个动作全落在**一枚牌子**上：
///
/// ```text
///   Register    挂上 / 改写自己的牌子        （行侧：我挂 / 我换）
///   Unregister  摘下自己的牌子，牌子留着
///   Query       照牌子上的字说出"在哪"        （问侧：我问 / 我答）
/// ```
///
/// **不存预约表**：谁能用哪个名字是装配期的事（谁拿到那枚入口），不是板子上的判定。
/// 故板只管一条判据：`vested_by(entry) == Some(who)`——**那枚入口是你亲手交给我的**。
/// 代价照实说：拿到持板者入口的任何服务都能挂**任意未被占用的名字**，"已占用"是唯一
/// 的护栏。
pub struct Board {
    signs: [Sign; Board::CAP],
    vested_by: VestedBy,
    unship: Unship,
}

impl Board {
    /// 板上有多少枚牌子。条数是策略，容器要有界。
    pub const CAP: usize = 16;

    /// 立一块板：两个注入的机制事实（探入口 / 放下）跟着板走——它们对每一枚牌子同值，
    /// 故不必逐个作参数传。
    pub const fn new(vested_by: VestedBy, unship: Unship) -> Board {
        Board {
            signs: [Sign::VACANT; Board::CAP],
            vested_by,
            unship,
        }
    }

    /// 挂上（或改写）自己的一枚牌子。返那枚入口。
    ///
    /// 四条判据，一条不多：
    ///
    /// - 那枚入口必须**是我亲手交出去的**（[`Fail::Denied`]）；
    /// - 已有牌子 ⇒ 名字**必须**与它一字不差（板不是改名处，改名就是换一枚牌：
    ///   先 [`Board::unregister`] 再登）；
    /// - 已有**别人**挂着的实例 ⇒ [`Fail::Taken`]；**自己**挂着的 ⇒ 覆盖（重登 / 换绑）；
    /// - 都没占 ⇒ 找一枚空牌子立上，板满则 [`Fail::Full`]。
    pub fn register(&mut self, name: Name, entry: PieToken, who: TaskId) -> Result<PieToken, Fail> {
        if (self.vested_by)(entry) != Some(who) {
            return Err(Fail::Denied);
        }
        match self.find(name) {
            Some(at) => {
                self.sweep_at(at);
                match self.signs[at].owner {
                    Some(owner) if owner != who => return Err(Fail::Taken),
                    _ => {}
                }
                // 换绑就是**先摘后挂**：上一枚入口是资源实体的一份引用，不放下就漏水。
                // 故这里与 `unregister` 走同一只手下牌。
                self.unship_at(at);
                self.signs[at].entry = Some(entry);
                self.signs[at].owner = Some(who);
                Ok(entry)
            }
            None => match self.find_vacant() {
                Some(at) => {
                    self.signs[at] = Sign {
                        name,
                        entry: Some(entry),
                        owner: Some(who),
                    };
                    Ok(entry)
                }
                None => Err(Fail::Full),
            },
        }
    }

    /// 摘下自己的一枚牌子（**牌子留着**：名字的位置不还给别人）。
    ///
    /// 摘的时候把板上那一份入口放下——它是资源实体的一份引用，不放下就漏水。
    ///
    /// **空牌子 = `Unknown`**：没有实例的牌子（从未登记 / 已注销 / 已死）答不出主人是谁，
    /// 故"这名字没人挂着"与"你挂的不是它"是两件事，`Denied` 只留给后者。
    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {
        let at = self.find(name).ok_or(Fail::Unknown)?;
        self.sweep_at(at);
        if self.signs[at].entry.is_none() {
            return Err(Fail::Unknown);
        }
        if self.signs[at].owner != Some(who) {
            return Err(Fail::Denied);
        }
        self.unship_at(at);
        Ok(())
    }

    /// 摘掉这位挂在板上的**全部**牌子，返摘了几枚（没有它的牌子 ⇒ `0`，**不算错**）。
    ///
    /// 与 [`Board::unregister`] 的分工：那一枚是**逐名**对偶的右半（一次一个名字，叫不出
    /// 名字就摘不下来）；这一句是**整位客人退场**——它挂过的名字一次全放下，故调用方不必
    /// 先知道它挂过哪些名字。两处走同一只手下牌（[`Board::unship_at`]），故"牌子留着、
    /// 名字不流转"这条规矩一字不差。
    pub fn evict(&mut self, who: TaskId) -> usize {
        let mut unshipped = 0;
        for at in 0..self.signs.len() {
            // `owner` 与 `entry` 同生同灭（`register` 一起写、`lift` 一起清）⇒ 认主人一格
            // 就够了：牌子还立着才数得进这一笔。
            if self.signs[at].owner == Some(who) {
                self.unship_at(at);
                unshipped += 1;
            }
        }
        unshipped
    }

    /// 照牌子上的字说出"在哪"。答不出只有两种可能，**且都是同一件事**：
    /// 板上没这一枚，或挂着的那一枚已经不在了。
    pub fn lookup(&mut self, name: Name) -> Result<PieToken, Fail> {
        self.lookup_after(name, |_| ())
    }

    /// 同 [`Board::lookup`]，但拿到入口后先交给 `ship`（适配层在这里把入口授给调用方，
    /// 免得"先查再授"中间再多一次查找）——`ship` 是那一手的名字（与 [`Unship`] 成对）。
    pub fn lookup_after(
        &mut self,
        name: Name,
        mut ship: impl FnMut(PieToken),
    ) -> Result<PieToken, Fail> {
        let at = self.find(name).ok_or(Fail::Unknown)?;
        self.sweep_at(at);
        match self.signs[at].entry {
            Some(entry) => {
                ship(entry);
                Ok(entry)
            }
            None => Err(Fail::Unknown),
        }
    }

    /// 板上此刻立着的牌子（诊断 / 将来的枚举：**只报有实例的**）。
    pub fn rows(&self) -> impl Iterator<Item = Sign> + '_ {
        self.signs.iter().copied().filter(Sign::standing)
    }

    /// 名字 → 牌子号。
    pub fn find(&self, name: Name) -> Option<usize> {
        self.signs
            .iter()
            .position(|sign| sign.named() && sign.name == name)
    }

    // ── 惰性剔除：板上不留死实例 ──────────────────────────────

    /// 扫一枚牌子：那一枚入口**答不出**（[`VestedBy`] 答 `None`——**不在我表里**与**门已封印**
    /// 是同一格）即**当场把牌子扫空**——不留死实例，也不留一个"实例已死"的中间状态。
    ///
    /// 读路径也扫（[`Board::lookup`] 会扫），故"已死"永远不会被答出去；代价是读也要
    /// 问一次表，那份代价由注入方定价（内核那侧是一次表查询）。
    fn sweep_at(&mut self, at: usize) {
        let Some(entry) = self.signs[at].entry else {
            return;
        };
        if (self.vested_by)(entry).is_none() {
            self.unship_at(at);
        }
    }

    /// 摘实例、清主人，**牌子留着**。
    fn unship_at(&mut self, at: usize) {
        if let Some(entry) = self.signs[at].entry {
            let _ = (self.unship)(entry);
        }
        self.signs[at].lift();
    }

    /// 空牌子号：**从未用过**的才算空（摘过牌的位子留着，名字的位置不流转）。
    fn find_vacant(&self) -> Option<usize> {
        self.signs.iter().position(|sign| !sign.named())
    }
}
// ── 两条读数（原先由宿主靶那七条用例钉着，用例随靶删了）────────────────
//
// **本文件今天没有一行测试**（用户裁定"protocol-case 没必要"：那台编外宿主靶与 `crates/gate`（已删）
// 的 `host` 那一门已删）。原先那七条里有两条是**读数的出处**，结论留在这里：
//
//   - [`Board::lookup`] 里那次 `sweep` 是**留着的**（见 `board/mod.rs` 末段）；
//   - [`Board::unregister`] 里那次 `sweep_at`：撤牌子也**先扫后判**。
//
// 三个注入点（`vested_by` / `unship`，见 [`Board::new`]）就是全部外部依赖。

// ── 两张会话失败域的对照表（原住 `protocol` 的 `system/board/call.rs`）────
//
// 两个入参都出自「约」（`session::core` 的 `Claim` / `Seat`）、产出的又是本文件自己的
// [`Fail`]，故它们与产出的那一格同住。`call.rs` 并进 `system/board/mod.rs` 那一刀
// 把这两张表落在这里。

/// 牌子上的名字（**定长解码面**：尾随 NUL 是填充，不是内容）。

pub fn map_claim(claim: Claim) -> Fail {
    match claim {
        Claim::Timeout => Fail::Unknown,
        Claim::Partial => Fail::Full,
    }
}

/// 装一条路的失败域 → 板的失败域。
///
/// 与 [`map_claim`] 同一条口径：名字/资源上的毛病（名字非法、同名已装、铸不出孔）是
/// **调用方写错了** ⇒ `Denied`；交不出去（对端已不在）⇒ `Unknown`（"它不在"）；
/// 账腾不出来 ⇒ `Full`。
pub fn map_seat(seat: Seat) -> Fail {
    match seat {
        Seat::NoName => Fail::Denied,
        Seat::NoHole => Fail::Denied,
        Seat::NoSeed => Fail::Unknown,
        Seat::Full => Fail::Full,
    }
}
