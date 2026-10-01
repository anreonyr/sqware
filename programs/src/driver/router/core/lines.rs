//! router::core::lines — **线那本账**：一张按线号索引的表、四个原语、几个查询。
//! ```text

use alloc::vec::Vec;

use protocol::communication::establish::Held;
use protocol::driver::line::Fail;
use runtime::env::mail::HolePie;

/// 一格：没主，或者有主（**那条路的持有者** ＋ 忙不忙）。
enum Cell {
    Idle,
    Owned { lane: Held, busy: bool },
}

/// 一张按线号索引的账。
/// ```text
///   格 = 空（这条线没主） | 有主 { 泊位, 忙 }
///   线号 = 下标（容量按 device_count 校验 ⇒ 越界不可表达）
/// ```
/// 另带一格**与格同长**的账：这条线**报过没有**（中断链上第一次领到它时打一行读数用）。
pub struct Lines {
    cells: Vec<Cell>,
    told: Vec<bool>,
}

impl Lines {
    /// 立账：容量按控制器自报的线数要（第 0 格永远空着——0 是"没有可领的"）。装不下 ⇒ `None`：
    /// 起域就拒，不留运行期分支。
    /// 两本账**同源同长**（都按 `device_count`）：报过没有那一格因此没有自己的容量。
    pub fn new(device_count: u32) -> Option<Lines> {
        let n = device_count as usize + 1;
        let mut cells = Vec::new();
        cells.try_reserve(n).ok()?;
        for _ in 0..n {
            cells.push(Cell::Idle);
        }
        let mut told = Vec::new();
        told.try_reserve(n).ok()?;
        told.resize(n, false);
        Some(Lines { cells, told })
    }

    pub fn occupy(&mut self, line: u32, lane: Held) -> Result<(), Fail> {
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

    pub fn deliver(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(Cell::Owned { lane, busy }) => {
                let at_peer = lane.tx().ok_or(Fail::Denied)?;
                match HolePie::from_token(at_peer).ring() {
                    Ok(()) => {}
                    Err(e) if e.source.is_busy() => {}
                    Err(_) => return Err(Fail::Denied),
                }
                *busy = true;
                Ok(())
            }
            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    pub fn exhaust(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(Cell::Owned { busy, .. }) => {
                *busy = false;
                Ok(())
            }
            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    pub fn vacate(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(cell @ Cell::Owned { .. }) => {
                *cell = Cell::Idle;
                Ok(())
            }
            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    pub fn lane(&self, line: u32) -> Option<&Held> {
        match self.cells.get(line as usize)? {
            Cell::Owned { lane, .. } => Some(lane),
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
    /// 报的是"中断链上第一次领到它"那一行读数——**一线一次**：反复来的中断不打第二行（否则
    /// 日志变脏），而 `vacate` 之后**也不清**（记的是"这一条线这一趟"，不是"这一位主人"）。
    /// **越界 ⇒ `false`**（不是"第一次"，也不是错误）。
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
