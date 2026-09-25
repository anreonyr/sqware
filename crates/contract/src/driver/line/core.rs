//! line::core — **账**：一张按线号索引的表、四个原语、几个查询、失败域。
//!
//! **四个原语**（`occupy` / `deliver` / `exhaust` / `vacate`）动账；**查询**（`lane` / `busy` /
//! `held` / `told`）只读账（`told` 顺手置一格"报过没有"，见它自己的注）。
//!
//! 本文件**不碰内核**——这条纪律现在由 **crate 边界**管着（见本 crate 头注）。唯一的设备侧动作
//! ——接线 / 静音 / 拆线——由适配层紧随原语之后做（它要动硬件），故核心只记账、可独立推理。

use alloc::vec::Vec;

use crate::session::Pier;

/// 四个原语会失败在哪一格。**一格对应一个不同的下一步**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 本控制器上没有这条线（没有主、或线号越出 `[1, device_count]`）⇒ 回头查树。
    Unknown,
    /// 这条线有人了 ⇒ 换个名字，或者等它 `vacate`。
    Taken,
    /// 那一帧推不出去（口封了 / 对端没了）⇒ 按"没投成"算，不置忙。
    Denied,
}

/// 一格：没主，或者有主（一条泊位 + 忙不忙）。
#[derive(Clone, Copy)]
enum Cell {
    Idle,
    Owned { lane: Pier, busy: bool },
}

/// 一张按线号索引的账。
///
/// ```text
///   格 = 空（这条线没主） | 有主 { 泊位, 忙 }
///   线号 = 下标（容量按 device_count 校验 ⇒ 越界不可表达）
/// ```
///
/// 另带一格**与格同长**的账：这条线**报过没有**（中断链上第一次领到它时打一行读数用）。
/// 它与占有无关——**一线一次，退场不清**（见 [`Lines::told`]）。
pub struct Lines {
    cells: Vec<Cell>,
    told: Vec<bool>,
}

impl Lines {
    /// 立账：容量按控制器自报的线数要（第 0 格永远空着——0 是"没有可领的"）。装不下 ⇒ `None`：
    /// 起域就拒，不留运行期分支。
    ///
    /// 两本账**同源同长**（都按 `device_count`）：报过没有那一格因此没有自己的容量。
    pub fn new(device_count: u32) -> Option<Lines> {
        let n = device_count as usize + 1;
        let mut cells = Vec::new();
        cells.try_reserve(n).ok()?;
        cells.resize(n, Cell::Idle);
        let mut told = Vec::new();
        told.try_reserve(n).ok()?;
        told.resize(n, false);
        Some(Lines { cells, told })
    }

    /// **occupy**：占住这一格（登记）。**接线那一手是它的后果**，由适配层紧随其后做；
    /// **拒绝（`Taken`）那一趟反过来**：刚交上来的那条泊位由适配层放下——账里根本没有它，
    /// 别人也不会替它收（`programs/src/driver/router/main.rs::drop_lane`）。
    pub fn occupy(&mut self, line: u32, lane: Pier) -> Result<(), Fail> {
        if line == 0 {
            return Err(Fail::Unknown);
        }
        match self.cells.get_mut(line as usize) {
            Some(cell @ Cell::Idle) => {
                *cell = Cell::Owned { lane, busy: false };
                Ok(())
            }
            Some(Cell::Owned { .. }) => Err(Fail::Taken),
            None => Err(Fail::Unknown),
        }
    }

    /// **deliver**：往这一格的泊位推一帧、**置忙**。推不出去 ⇒ 不置忙（那一帧没送到）。
    pub fn deliver(&mut self, line: u32, frame: &[u8]) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(Cell::Owned { lane, busy }) => {
                lane.post(frame).map_err(|()| Fail::Denied)?;
                *busy = true;
                Ok(())
            }
            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    /// **exhaust**：排空——这一格回闲。**放回那一手是它的后果**，由适配层紧随其后做。
    pub fn exhaust(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(Cell::Owned { busy, .. }) => {
                *busy = false;
                Ok(())
            }
            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    /// **vacate**：主人没了——空出这一格。**拆线那一手是它的后果**，由适配层紧随其后做。
    pub fn vacate(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(cell @ Cell::Owned { .. }) => {
                *cell = Cell::Idle;
                Ok(())
            }
            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    /// 这一格的泊位（没主 ⇒ `None`）。
    pub fn lane(&self, line: u32) -> Option<Pier> {
        match self.cells.get(line as usize)? {
            Cell::Owned { lane, .. } => Some(*lane),
            Cell::Idle => None,
        }
    }

    /// 手边还压着哪几条（忙的那些）——**放回那一拍按它走**。
    pub fn busy(&self) -> impl Iterator<Item = u32> + '_ {
        self.cells.iter().enumerate().filter_map(|(i, c)| match c {
            Cell::Owned { busy: true, .. } => Some(i as u32),
            _ => None,
        })
    }

    /// 有主的那些条（探活用：答不出的那一条该 `vacate`）。
    pub fn held(&self) -> impl Iterator<Item = u32> + '_ {
        self.cells.iter().enumerate().filter_map(|(i, c)| match c {
            Cell::Owned { .. } => Some(i as u32),
            _ => None,
        })
    }

    /// **told**：这条线**报过没有**？（第一次 ⇒ 置上并答 `true`）
    ///
    /// 报的是"中断链上第一次领到它"那一行读数——**一线一次**：反复来的中断不打第二行（否则
    /// 日志变脏），而 `vacate` 之后**也不清**（记的是"这一条线这一趟"，不是"这一位主人"）。
    /// **越界 ⇒ `false`**（不是"第一次"，也不是错误）。
    ///
    /// **照实记**：这一格从前是 router 自己另开的一本定长账（`[u64; 2]`，0..127 号线），容量与
    /// 本账**不联动**，越界是**裸下标** ⇒ 控制器自报 > 127 条线的机器上第 128 条当场 panic。
    /// 并进账里之后两本同源同长，"越界不可表达"这条口径对它也成立。
    pub fn told(&mut self, line: u32) -> bool {
        match self.told.get_mut(line as usize) {
            Some(seen) => {
                let fresh = !*seen;
                *seen = true;
                fresh
            }
            None => false,
        }
    }
}
