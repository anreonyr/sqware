//! hub::core — **设备那一本账**：册上一条一条 ＋ 每类那枚盟。**纯核心**：不碰内核、不碰树、
//! 不碰会话——喂两个假闭包（"主人还答得出吗"）就能把规矩推理干净。
//!
//! ```text
//!   Entry   一台：名 ＋ 类 ＋ 区 ＋ 线 ＋ **那一枚**（hub 手里那一枚：那一台那一份孔）
//!   Cell    格 = Vacant(Entry) | Held { entry, owner }      ← "有主没主"是一格
//!   League  类 → 盟（hub 立的那一枚）
//! ```
//!
//! # 三条不变量（做进类型，不写成注释故事）
//!
//! 1. **一台一主，且只认活着的主人**——`Cell` 两形说的是"有没有主"；"有主但主人没了"当场
//!    由 [`Ledger::claim`] / [`Ledger::vacate`] 收掉，不留成第三种形状。
//! 2. **名在册里唯一**——[`Ledger::enroll`] 撞名即拒（调用方断言：这台机器的自述坏了）。
//! 3. **类 → 盟一处**——[`Ledger::league`] 是唯一那一处；写 permit 与代报名都从它取。
//!
//! # 它不认识"哪一枚孔是几号"
//!
//! **哪一台是"哪一枚孔响了"**回答的（[`Ledger::claim`] 收的是那一枚的号，不是名字）——这正是
//! "驱动自己去 `/dev` 拿"在核心这一侧的落点：请求里没有"哪一件"这一格。

use alloc::string::String;
use alloc::vec::Vec;

use env::{PieToken, TaskId};
use protocol::driver::hub::{Fail, LIST_MAX, Window};
use protocol::system::coalition::CoalitionId;

// ── 一台与它的主人 ──────────────────────────────────────────

/// **一台设备在 hub 账上是什么**：名（树上的坐标）、类（认领的口子）、线（区→线那条权威在
/// hub）、**那一页**（认领时授出去的门闩）、**那一枚孔**（hub 为这一台铸的、挂在 `/dev` 那一
/// 格上的那一份）。
///
/// **照实记（"坐标"那一格退场了）**：这一格原先还有一枚 `Key`（区）——它只有一个读者，就是
/// 起手那一趟"**把装配者推来的记录对上台**"（记录给的是坐标 ＋ 号，而名 / 类 / 线只有树说得
/// 清）。那一趟在**造出这一行之前**就用完了它（对上了才造这一行）⇒ 账里再留一枚没人读的
/// `Key` 就是死格。**机制退了，格也退**。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub name: String,
    pub class: String,
    pub line: u32,
    /// 那一页（引导域交过来的设备门闩）——认领成功时授给主人的就是它。
    pub page: PieToken,
    /// hub 为这一台铸的那一枚孔——**就是你 `find` 到的那一格上挂的那一份**。
    /// 「哪一台」由"哪一枚孔响了"回答（[`Ledger::claim`] 收的正是它）。
    pub door: PieToken,
}

/// **这台的主人**：号 ＋ **主人那一枚**（他铸的报活孔）。
///
/// 内核那一问（`UnitCall::Join`）只许同队或父域，hub 与驱动是兄弟 ⇒ 探活只能问主人自己交来的
/// 那一枚（`mail::reserve`，与线路由者那条同一手）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Owner {
    pub task: TaskId,
    pub sensor: PieToken,
}

/// **一格**：空着，或有主。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Cell {
    /// 没主（随时可认）。
    Vacant(Entry),
    /// 有主。
    Held { entry: Entry, owner: Owner },
}

impl Cell {
    /// 这一格那一台（两形都有）。
    pub fn entry(&self) -> &Entry {
        match self {
            Cell::Vacant(entry) => entry,
            Cell::Held { entry, .. } => entry,
        }
    }

    /// 这一格有主吗。
    pub fn held(&self) -> bool {
        matches!(self, Cell::Held { .. })
    }
}

/// 一类 → 它那枚盟。
#[derive(Clone, PartialEq, Eq, Debug)]
struct League {
    class: String,
    coalition: CoalitionId,
}

// ── 册 ──────────────────────────────────────────────────────

/// 设备那一本账。
pub struct Ledger {
    cells: Vec<Cell>,
    leagues: Vec<League>,
}

impl Ledger {
    /// 立一份空册。
    pub const fn new() -> Ledger {
        Ledger {
            cells: Vec::new(),
            leagues: Vec::new(),
        }
    }

    // ── 入册 ────────────────────────────────────────────────

    /// 册 · 写：收一台。**撞名即拒**（名是这一台的坐标，两处写同一个名＝自述坏了）。
    ///
    /// 备不下那一行也答 `Err`：装配期那一趟把它折成"入册没成"，与撞名同一个下一步
    /// （这一台机器起不来，而不是"少收一台"）。
    pub fn enroll(&mut self, entry: Entry) -> Result<(), ()> {
        if self.cells.iter().any(|c| c.entry().name == entry.name) {
            return Err(());
        }
        self.cells.try_reserve(1).map_err(|_| ())?;
        self.cells.push(Cell::Vacant(entry));
        Ok(())
    }

    // ── 类 → 盟 ─────────────────────────────────────────────

    /// 册 · 写：这一类那枚盟；没铸过就铸（`mint` 由适配层给——核心不叫盟册）。
    ///
    /// **一处定义**：写 permit 与"代报名"都从这里取，故同一类不会有两枚盟。
    pub fn league(&mut self, class: String, mint: impl FnOnce() -> CoalitionId) -> CoalitionId {
        if let Some(league) = self.leagues.iter().find(|l| l.class == class) {
            return league.coalition;
        }
        let coalition = mint();
        // 备不下就只有不记（下一次问会再铸一枚——**代价照实记**：那种时候这台机器已经不对了，
        // 而"静默少一件事"比"当场崩"更像这台仓的失败法）。
        if self.leagues.try_reserve(1).is_ok() {
            self.leagues.push(League { class, coalition });
        }
        coalition
    }

    /// 册 · 读：这一类那枚盟（没铸过 ⇒ `None`）。
    pub fn coalition_of(&self, class: String) -> Option<CoalitionId> {
        self.leagues
            .iter()
            .find(|l| l.class == class)
            .map(|l| l.coalition)
    }

    // ── 认领 / 空出 ─────────────────────────────────────────

    /// 册 · 写：认领那一台（**`token` = 哪一枚孔响了**，故请求里没有"哪一件"这一格）。
    ///
    /// 三支：
    /// - 空着 ⇒ 写主人，交出那一台（调用方据此授门闩、写 permit、回契）；
    /// - 有主、主人还答得出（`alive`）⇒ [`Fail::Taken`]；
    /// - 有主、主人答不出了 ⇒ **当场接手**（顺手收掉再写，不留脏格）。
    ///
    /// 没这一枚孔 ⇒ [`Fail::Unknown`]（这一枚不是本册的）。
    pub fn claim(
        &mut self,
        door: PieToken,
        owner: Owner,
        alive: impl Fn(PieToken) -> bool,
    ) -> Result<Entry, Fail> {
        let Some(at) = self.cells.iter().position(|c| c.entry().door == door) else {
            return Err(Fail::Unknown);
        };
        if let Cell::Held { owner: held, .. } = &self.cells[at] {
            if alive(held.sensor) {
                return Err(Fail::Taken);
            }
        }
        let entry = self.cells[at].entry().clone();
        self.cells[at] = Cell::Held {
            entry: entry.clone(),
            owner,
        };
        Ok(entry)
    }

    /// 册 · 写：空出主人没了的那几格。返**空出了几格**（读数）。
    ///
    /// **只由 hub 自己调**（探活看出来）——常驻驱动不退场，退场＝死 ⇒ "显式放回"那一格不造。
    pub fn vacate(&mut self, alive: impl Fn(PieToken) -> bool) -> usize {
        let mut freed = 0;
        for cell in self.cells.iter_mut() {
            let Cell::Held { entry, owner } = cell else {
                continue;
            };
            if alive(owner.sensor) {
                continue;
            }
            *cell = Cell::Vacant(entry.clone());
            freed += 1;
        }
        freed
    }

    // ── 读数 ────────────────────────────────────────────────

    /// 册 · 读：这一类从 `from` 起的一窗（名字 ＋ 有主那一位掩码）。
    ///
    /// **越界答空窗**（诚实的答案，不是错误——与盟册 `bloc` 同一条口径）。
    pub fn list(&self, class: String, from: u32) -> Window {
        let mut window = Window::EMPTY;
        window.from = from;
        let mut slot = 0usize;
        for (i, cell) in self
            .cells
            .iter()
            .filter(|c| c.entry().class == class)
            .enumerate()
        {
            if (i as u32) < from {
                continue;
            }
            if slot >= LIST_MAX {
                break;
            }
            window.names[slot] = cell.entry().name.clone();
            if cell.held() {
                window.held[slot / 8] |= 1 << (slot % 8);
            }
            slot += 1;
        }
        window.n = slot as u8;
        window
    }

    /// 册 · 读：**这一类怎么落**——那几台的（名 ＋ 那枚孔），序 = 入册序（同 [`Ledger::list`]）。
    ///
    /// **它有一个读者**：hub 落 `/dev/<类>` 那一趟（[`bridge::land`] 收的是切片，故调用方
    /// 自己收一下）——以及挂组那一趟（把每一枚孔挂进那只组）。
    ///
    /// **为什么不是从 [`Ledger::list`] 推**：那一窗给的是名字 ＋ 有主掩码（客人的读数），
    /// 而这一手给的是**孔**——落格与挂组要的正是"哪一枚孔是这一台的"。
    ///
    /// [`bridge::land`]: crate::system::operator::bridge::land
    pub fn doors(&self, class: String) -> impl Iterator<Item = (&String, PieToken)> + '_ {
        self.cells
            .iter()
            .filter(move |c| c.entry().class == class)
            .map(|c| (&c.entry().name, c.entry().door))
    }

    /// 册 · 读：册上出现过的**类**（入册序，去重）——hub 逐类立盟、逐类落 `/dev` 要它。
    ///
    /// 返 `None` = 备不下（调用方按"这一台起不来"处置：它是起手那一步）。
    pub fn classes(&self) -> Option<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for cell in &self.cells {
            let class = cell.entry().class.clone();
            if out.contains(&class) {
                continue;
            }
            out.try_reserve(1).ok()?;
            out.push(class);
        }
        Some(out)
    }

    /// 册 · 读：册上有几台（读数用）。
    pub fn count(&self) -> usize {
        self.cells.len()
    }
}
