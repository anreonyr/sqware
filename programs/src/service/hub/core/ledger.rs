//! Device inventory, class admission, execution and binding responsibilities.

use alloc::string::String;
use alloc::vec::Vec;

use env::{PieToken, TaskId};
use hub_api::{Fail, INVENTORY_MAX, LIST_MAX, Window};

use super::{Cell, Entry, Owner, league::League};

/// 设备那一本账：普通设备的格子 ＋ 每类那枚盟 ＋ **DMA 安全绑定的协调记录**
/// （不新增第二本设备账：设备、认领记录与绑定进度同住这一本）。
pub struct Ledger {
    pub(super) cells: Vec<Cell>,
    pub(super) leagues: Vec<League>,
    /// DMA 安全绑定：一笔一条，序号不复用。
    pub(super) bindings: Vec<super::binding::Binding>,
    pub(super) executions: Vec<super::Execution>,
    /// 绑定编号的游标（从 1 起，不复用）。
    pub(super) serial: u64,
    /// 本册那一台（绑定编号里的 `hub` 那一格）。
    pub(super) hub: TaskId,
    pub(super) settling: bool,
}

impl Ledger {
    /// 立一份空册
    pub const fn new() -> Ledger {
        Ledger {
            cells: Vec::new(),
            leagues: Vec::new(),
            bindings: Vec::new(),
            executions: Vec::new(),
            serial: 0,
            hub: TaskId::new(0),
            settling: false,
        }
    }

    /// 本册那一台是谁（绑定编号要用）。
    pub fn with_hub(mut self, hub: TaskId) -> Self {
        self.hub = hub;
        self
    }

    /// 册 · 写：收一台。**撞名即拒**（名是这一台的坐标，两处写同一个名＝自述坏了）
    /// （这一台机器起不来，而不是"少收一台"）
    pub fn enroll(&mut self, entry: Entry) -> Result<(), ()> {
        if self.cells.len() >= INVENTORY_MAX
            || self.cells.iter().any(|c| c.entry().name == entry.name)
        {
            return Err(());
        }
        self.cells.try_reserve(1).map_err(|_| ())?;
        self.cells.push(Cell::Vacant { entry, origin: None });
        Ok(())
    }

    /// 交付成功后才登记新主人；失败保留原账。
    pub fn claim_with<T>(
        &mut self,
        door: PieToken,
        owner: Owner,
        alive: impl Fn(PieToken) -> bool,
        deliver: impl FnOnce(&Entry) -> Result<T, Fail>,
    ) -> Result<(Entry, T), Fail> {
        let Some(at) = self.cells.iter().position(|c| c.ready() && c.entry().door == door) else {
            return Err(Fail::Unknown);
        };
        if let Cell::Held { owner: held, .. } = &self.cells[at] {
            if alive(held.sensor) {
                return Err(Fail::Taken);
            }
        }
        let entry = self.cells[at].entry().clone();
        let delivered = deliver(&entry)?;
        self.cells[at] = Cell::Held {
            entry: entry.clone(),
            origin: self.cells[at].device(),
            owner,
        };
        Ok((entry, delivered))
    }

    /// 册 · 写：空出主人没了的那几格。返**空出了几格**（读数）
    pub fn vacate(&mut self, alive: impl Fn(PieToken) -> bool) -> usize {
        let mut freed = 0;
        for cell in self.cells.iter_mut() {
            let Cell::Held { entry, origin, owner } = cell else {
                continue;
            };
            if alive(owner.sensor) {
                continue;
            }
            *cell = Cell::Vacant { entry: entry.clone(), origin: *origin };
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
            .filter(|c| c.ready() && c.entry().class == class)
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
    pub fn doors(&self, class: String) -> impl Iterator<Item = (&String, PieToken)> + '_ {
        self.cells
            .iter()
            .filter(move |c| c.ready() && c.entry().class == class)
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
