//! router::core::lines — **线那本账**：一张按线号索引的表、四个原语、几个查询。
//!
//! ```text
//!   occupy   占住这一格 + （适配层紧随其后）接线
//!   deliver  往这一格的泊位推一帧 + 置忙
//!   exhaust  排空：这一格回闲 + （适配层）放回
//!   vacate   主人没了：空出这一格 + （适配层）拆线
//! ```
//!
//! **照实记（它原先住 `protocol::driver::line::core`）**：那一份的读者只有本域一处——按
//! [`crate::driver`] 的判据（"只有持有那台设备的人读得到它吗"），它是**线路由者的内部管理表**，
//! 不是两侧共享的语言。故搬回持有它的人这里：协议那一侧只留**形与码**
//! （`protocol::driver::line::frame`）与**客侧几手**（`protocol::driver::line::client`）。
//!
//! **两侧之间的约定**（"权威与属主"、"静音还是压着不结"、"线 = 区的函数"）仍写在
//! `protocol::driver::line` 那一份正文里——那几节讲的是约定，不是这本账的形状。
//!
//! **它碰内核的地方只有一处**：`deliver` 推那一帧（`mail::HolePie::from_token(..).push(..)`）。
//! **设备侧的动作**（接线 / 静音 / 拆线）一律不在这里：它们由适配层紧随原语之后做（那几手要动
//! 硬件），故账仍可独立推理。

use alloc::vec::Vec;

use protocol::communication::establish::Held;
use protocol::driver::line::Fail;
use runtime::env::mail;

/// 一格：没主，或者有主（**那条路的持有者** ＋ 忙不忙）。
///
/// **装的是 [`Held`]**（不是 `Endpoint`）：这一格的关系是**真·作用域寿命**——"这一格归这位
/// 主人"就是它活着的全部理由，主人没了（`vacate`）或换一条线占进来，那一格就该被放下。
/// 放下一手因此由 `Held` 的 `Drop` 代劳，**一处也不用记**（旧形状里那一手是适配层自己写的
/// `drop_lane` / `unseat`）。
enum Cell {
    Idle,
    Owned { lane: Held, busy: bool },
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
        // 逐格压（不是 `resize`）：这一格装的是持有者，不是 `Copy` 的号。
        for _ in 0..n {
            cells.push(Cell::Idle);
        }
        let mut told = Vec::new();
        told.try_reserve(n).ok()?;
        told.resize(n, false);
        Some(Lines { cells, told })
    }

    /// **occupy**：占住这一格（登记）。**接线那一手是它的后果**，由适配层紧随其后做；
    /// **拒绝（`Taken`）那一趟反过来**：刚交上来的那条路**不在账里**——`lane` 是按值收进来的
    /// （[`Held`]），`Taken` / `Unknown` 两条出口都把它丢在门外 ⇒ **它自己放下**，
    /// 调用方一行都不用写。
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

    /// **deliver**：往这一格的泊位推一帧、**置忙**。推不出去 ⇒ 不置忙（那一帧没送到）。
    pub fn deliver(&mut self, line: u32, frame: &[u8]) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(Cell::Owned { lane, busy }) => {
                // 推的是**对端那一枚**（我写、对端读）；还没认到 ⇒ 与从前 `Pier::post`
                // 自己那一格同一落点：没写端就发不出去。
                let at_peer = lane.tx().ok_or(Fail::Denied)?;
                mail::HolePie::from_token(at_peer)
                    .push(frame)
                    .map_err(|_| Fail::Denied)?;
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

    /// **vacate**：主人没了——空出这一格（**放下那条路**：格子换回 `Idle` 即 `Held` 落出
    /// 作用域，本端铸的那一枚随之放下）。**拆线那一手是它的后果**，由适配层紧随其后做
    /// （先拆线再空格：那一枚还挂在组上）。
    pub fn vacate(&mut self, line: u32) -> Result<(), Fail> {
        match self.cells.get_mut(line as usize) {
            Some(cell @ Cell::Owned { .. }) => {
                *cell = Cell::Idle;
                Ok(())
            }
            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    /// 这一格的泊位（没主 ⇒ `None`）。**借**出去：持有者不在这一层放手（放下归格子自己）。
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
