//! line::core — **账**：一张按线号索引的表、四个原语、失败域。
//!
//! 本文件**不碰内核**（不出现 `runtime::`）：唯一的设备侧动作——接线 / 静音 / 拆线——由适配层
//! 紧随原语之后做（它要动硬件），故核心只记账、可独立推理。

use alloc::vec::Vec;

use crate::session::Pier;

/// 四个原语会失败在哪一格。**一格对应一个不同的下一步**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 本控制器上没有这条线（没有主、或线号越出 `[1, ndev]`）⇒ 回头查树。
    Unknown,
    /// 这条线有人了 ⇒ 换个名字，或者等它 `vacate`。
    Taken,
    /// 那一帧推不出去（口封了 / 对端没了）⇒ 按"没投成"算，不置忙。
    Denied,
}

/// 一格：没主，或者有主（一条泊位 + 忙不忙）。
#[derive(Clone, Copy)]
enum Cell {
    Free,
    Owned { lane: Pier, busy: bool },
}

/// 一张按线号索引的账。
///
/// ```text
///   格 = 空（这条线没主） | 有主 { 泊位, 忙 }
///   线号 = 下标（容量按 ndev 校验 ⇒ 越界不可表达）
/// ```
pub struct Lines {
    cells: Vec<Cell>,
}

impl Lines {
    /// 立账：容量按控制器自报的线数要（第 0 格永远空着——0 是"没有可领的"）。装不下 ⇒ `None`：
    /// 起域就拒，不留运行期分支。
    pub fn new(ndev: u32) -> Option<Lines> {
        let mut cells = Vec::new();
        cells.try_reserve(ndev as usize + 1).ok()?;
        cells.resize(ndev as usize + 1, Cell::Free);
        Some(Lines { cells })
    }

    /// **occupy**：占住这一格（登记）。**接线那一手是它的后果**，由适配层紧随其后做。
    pub fn occupy(&mut self, line: u32, lane: Pier) -> Result<(), Fail> {
        if line == 0 {
            return Err(Fail::Unknown);
        }
        match self.cells.get_mut(line as usize) {
            Some(cell @ Cell::Free) => {
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
            Some(Cell::Free) | None => Err(Fail::Unknown),
        }
    }

    /// **exhaust**：排空——这一格回闲。**放回那一手是它的后果**，由适配层紧随其后做。
    pub fn exhaust(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(Cell::Owned { busy, .. }) => {
                *busy = false;
                Ok(())
            }
            Some(Cell::Free) | None => Err(Fail::Unknown),
        }
    }

    /// **vacate**：主人没了——空出这一格。**拆线那一手是它的后果**，由适配层紧随其后做。
    pub fn vacate(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(cell @ Cell::Owned { .. }) => {
                *cell = Cell::Free;
                Ok(())
            }
            Some(Cell::Free) | None => Err(Fail::Unknown),
        }
    }

    /// 这一格的泊位（没主 ⇒ `None`）。
    pub fn lane(&self, line: u32) -> Option<Pier> {
        match self.cells.get(line as usize)? {
            Cell::Owned { lane, .. } => Some(*lane),
            Cell::Free => None,
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
}
