//! board::core — **板那一本账**：定长的一叠牌子 ＋ 按名字 / 按主人的几条读。
//!
//! **照实记（它原先住 `protocol::system::board::core`）**：那一份的读者只有本域的持板线程
//! （`prog-board` 那一段）——按"协议 = 共享语言"的判据，它属于实现侧。协议那一侧只留
//! **失败域**（`Fail`）与几具客手。
//!
//! 本文件只讲牌子与那五条动作；**板为什么就一枚线程、惰性剔除的口径**写在协议那一边
//! （`protocol::system::board` 的正文）。

use env::{Name, PieToken, TaskId};

use protocol::communication::establish::vested_by;
use protocol::system::board::Fail;
use runtime::env::mail;

// ── 结构 ────────────────────────────────────────────────────

/// 板上的一枚牌子：**叫什么、在哪、谁挂的**。
///
/// 入口是 [`Option`] 而不是哨兵值：**"没有实例"是一个状态**（从未登记 / 已注销 / 已死
/// 三者同一个状态），故 `Unknown` 只有一种语义。
///
/// `owner` 是挂牌人：它在**有实例**时才有意义，而实例一死 `sweep_at` 就把牌子扫空
/// （连 `owner` 一起）——**故它不可能过期**，不需要第二套"owner 还在吗"的规则。
///
/// **它不外露**（残枝那一刀）：唯一的观察口 [`Board::rows`] 与 `Sign::{owner, standing}`
/// 全仓零读者，故一并删掉——牌子是本账的内部格，外面只经 [`Board::lookup`] 问"这一枚在哪"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Sign {
    name: Name,
    entry: Option<PieToken>,
    owner: Option<TaskId>,
}

impl Sign {
    /// 空牌子：板子的初值。
    const VACANT: Sign = Sign {
        name: Name::EMPTY,
        entry: None,
        owner: None,
    };

    /// 板上有没有这一枚的名字（诊断：牌子存在，哪怕此刻是空的）。
    fn named(&self) -> bool {
        !self.name.is_empty()
    }

    fn lift(&mut self) {
        self.entry = None;
        self.owner = None;
    }
}

// **照实记（"放下"那一枚函数指针已经退场，末了那一具壳也退了）**：它从前是一个注入的别名
// （`pub type Unship = fn(PieToken) -> Result<(), ()>`），由 `board()` 接上身体；身体只有
// 一个（`mail::release`）⇒ 这里直接叫，别名与构造点一并撤掉。中间那一版还剩一具
// `establish::unship` 的转发壳，它也与 `establish` 那六具一起删了（见那个文件的照实记）。

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
}

impl Board {
    /// 板上有多少枚牌子。条数是策略，容器要有界。
    pub const CAP: usize = 16;

    /// 立一块板。**两枚机制事实不进这里**（照实记）：从前它们跟板走（`Board { vested_by,
    /// unship }`），故多出两枚类型别名、一个构造点、一层"谁接上"；而两边各只有一个身体
    /// （[`vested_by`] 与 `mail::release`）⇒ 直接叫。
    pub const fn new() -> Board {
        Board {
            signs: [Sign::VACANT; Board::CAP],
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
        if vested_by(entry) != Some(who) {
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
    /// 免得"先查再授"中间再多一次查找）——`ship` 是那一手的名字（与"放下"成对）。
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

    /// 名字 → 牌子号。
    pub fn find(&self, name: Name) -> Option<usize> {
        self.signs
            .iter()
            .position(|sign| sign.named() && sign.name == name)
    }

    // ── 惰性剔除：板上不留死实例 ──────────────────────────────

    /// 扫一枚牌子：那一枚入口**答不出**（[`vested_by`] 答 `None`——**不在我表里**与**门已封印**
    /// 是同一格）即**当场把牌子扫空**——不留死实例，也不留一个"实例已死"的中间状态。
    ///
    /// 读路径也扫（[`Board::lookup`] 会扫），故"已死"永远不会被答出去；代价是读也要
    /// 问一次表，那份代价由注入方定价（内核那侧是一次表查询）。
    fn sweep_at(&mut self, at: usize) {
        let Some(entry) = self.signs[at].entry else {
            return;
        };
        if vested_by(entry).is_none() {
            self.unship_at(at);
        }
    }

    /// 摘实例、清主人，**牌子留着**。
    fn unship_at(&mut self, at: usize) {
        if let Some(entry) = self.signs[at].entry {
            let _ = mail::release(entry);
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
// 全部外部依赖只有两处：`vested_by`（探入口）与 `mail::release`（放下）——直接叫的那两具身体。
