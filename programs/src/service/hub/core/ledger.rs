//! 一台（Entry）· 它的主人（Owner）· 一格（Cell）·
//! 册本身（Ledger）与册上那几手（收 / 认领 / 空出 / 取窗 / 落格要的那几件）。

use alloc::string::String;
use alloc::vec::Vec;

use env::{PieToken, TaskId};
use protocol::service::hub::{Fail, LIST_MAX, Window};

use super::league::League;

/// **一台设备在 hub 账上是什么**：名（树上的坐标）、类（认领的口子）、线（区→线那条权威在
/// hub）、**那一页**（认领时授出去的门闩）、**那一枚孔**（hub 为这一台铸的、挂在 `/dev` 那一
/// 格上的那一份）
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub name: String,
    pub class: String,
    pub line: u32,
    /// 那一页（装配者交过来的设备门闩）——认领成功时授给主人的就是它
    pub page: PieToken,
    /// hub 为这一台铸的那一枚孔——**就是你 `find` 到的那一格上挂的那一份**
    /// 「哪一台」由"哪一枚孔响了"回答（Ledger::claim 收的正是它）
    pub door: PieToken,
}

/// **这台的主人**：号 ＋ **主人那一枚**（他铸的报活孔）
/// 内核那一问（UnitCall::Join）只许同队或父域，hub 与驱动是兄弟 ⇒ 探活只能问主人自己交来的
/// 那一枚（mail::reserve，与线路由者那条同一手）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Owner {
    pub task: TaskId,
    pub sensor: PieToken,
}

/// **一格**：空着，或有主
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Cell {
    /// 没主（随时可认）
    Vacant(Entry),
    /// 有主
    Held { entry: Entry, owner: Owner },
}

impl Cell {
    pub fn entry(&self) -> &Entry {
        match self {
            Cell::Vacant(entry) => entry,
            Cell::Held { entry, .. } => entry,
        }
    }

    pub fn held(&self) -> bool {
        matches!(self, Cell::Held { .. })
    }
}

/// 设备那一本账
pub struct Ledger {
    cells: Vec<Cell>,
    pub(super) leagues: Vec<League>,
}

impl Ledger {
    /// 立一份空册
    pub const fn new() -> Ledger {
        Ledger {
            cells: Vec::new(),
            leagues: Vec::new(),
        }
    }

    /// 册 · 写：收一台。**撞名即拒**（名是这一台的坐标，两处写同一个名＝自述坏了）
    /// （这一台机器起不来，而不是"少收一台"）
    pub fn enroll(&mut self, entry: Entry) -> Result<(), ()> {
        if self.cells.iter().any(|c| c.entry().name == entry.name) {
            return Err(());
        }
        self.cells.try_reserve(1).map_err(|_| ())?;
        self.cells.push(Cell::Vacant(entry));
        Ok(())
    }

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

    /// 册 · 写：空出主人没了的那几格。返**空出了几格**（读数）
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

    /// 册 · 读：这一类从 `from` 起的一窗（名字 ＋ 有主那一位掩码）
    /// **越界答空窗**（诚实的答案，不是错误——与盟册 `bloc` 同一条口径）
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

    /// 册 · 读：**这一类怎么落**——那几台的（名 ＋ 那枚孔），序 = 入册序（同 Ledger::list）
    /// **为什么不是从 Ledger::list 推**：那一窗给的是名字 ＋ 有主掩码（客人的读数）
    /// crate::system::publication::land
    pub fn doors(&self, class: String) -> impl Iterator<Item = (&String, PieToken)> + '_ {
        self.cells
            .iter()
            .filter(move |c| c.entry().class == class)
            .map(|c| (&c.entry().name, c.entry().door))
    }

    /// 册 · 读：册上出现过的**类**（入册序，去重）——hub 逐类立盟、逐类落 `/dev` 要它
    /// 返 `None` = 备不下（调用方按"这一台起不来"处置：它是起手那一步）
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

    /// 册 · 读：册上有几台（读数用）
    pub fn count(&self) -> usize {
        self.cells.len()
    }
}
