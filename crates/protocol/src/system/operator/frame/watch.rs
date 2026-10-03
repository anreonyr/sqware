//! **事件那一支**：树上"变了什么"记下来之后的那一形。
//!
//! # 它不在 `Req` / `Wire` 那两个聚合里，也不上线
//! `road.rs` 是**一问一答那一族**（`Req` / `Wire` / `Union`）：那些帧都要过孔、都要一个对面。
//! 事件不走孔——它由持树者**直接写进订阅者那一页**（`communication::rack`），故：
//! - **没有动作码那一格**（线上没有"哪条报文"可认，是写进内存的一段记录）；
//! - **进不了 `Req` / `Wire`**（那两张表是"问话与答话的形状"，事件既不是问也不是答）。
//!
//! 一条规矩，加帧时照它写：**上线的帧住 `road.rs`，不上线的记录住这里。**
//!
//! # 一条事件是什么
//! `{ kind, road, id, owner }`——**一次改动的全坐标**：那一条路（从根写起）此刻可寻址、
//! 那一格自己的号、以及它的主人。订阅者按 `road` 做前缀过滤，故**前缀里那几格的幂等落法
//! 不单独报**：报了也只会是"这条路可寻址"的同义句。

use env::wire::Field as _;
use env::TaskId;

use crate::common::path::{Path, PathBuf};
use crate::wire::message::Message;

use super::vocab::EntryId;
use env::wire::Span as _;

/// 一次改动是**哪一种**。三个"真动了树"的下场 ＋ 一个"只改了归属"的下场（换绑不动号）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 新铸一枚砖（`land` 那一手真造了一格）
    Landed = 1,
    /// **换绑**：那一格已占，这一手把它的两轴改掉了（**号不动**）——不冒充"新铸"
    Rebound = 2,
    /// 新立一块窗格（`part`）；已是窗格的幂等那一档**不报**（树一个字节没变）
    Parted = 3,
    /// 剪掉一格（`trim`）；剪了个空的（碑）**不报**
    Trimmed = 4,
}

impl Kind {
    /// 线上那一字节 → 那一种。**表外的记 ⇒ 读不懂**（不猜：一个不认识的事件比没有更坏）
    pub fn of(raw: u8) -> Option<Kind> {
        Some(match raw {
            1 => Kind::Landed,
            2 => Kind::Rebound,
            3 => Kind::Parted,
            4 => Kind::Trimmed,
            _ => return None,
        })
    }

    /// 那一种 → 线上那一字节
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// **一条事件**：改动的全坐标 ＋ **它自己的号**。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    /// **这一条事件的号**（持树者那一具架自己的计数，从 1 起，与环里的格一一对应）。
    ///
    /// 它为什么在载荷里（而不靠环那一格的头）：手递出去的是**那一格的载荷**，而环会绕回来
    /// ——同一条手取到手的可能是**更新的内容**。有了这一格，读者能自己说清"中间丢了几条"
    /// （号跳了）与"这一条我读过了"（号没前进）。
    pub seq: u64,
    pub kind: Kind,
    /// 那一条路（**从根写起**）
    pub road: PathBuf,
    /// 那一格自己的号
    pub id: EntryId,
    /// 那一格的主人（`Mine::Yes` 那一档落下的就是它；无主 = 零号）
    pub owner: TaskId,
}

/// **记录那一形**（`env::Frame`：字段表就是结构体，偏移由各格求和）。
///
/// 与线上帧同一条手，只是为了"往那一页里写一段自描述记录"——`kind` 那一格既让读者认得出
/// 是哪一种改动，也让**表外的 kind 整条读不懂**（与 `Req` 的动作码同一条纪律）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct EventFrame {
    pub seq: u64,
    pub kind: u8,
    pub road: PathBuf,
    pub id: EntryId,
    pub owner: TaskId,
}

const _: () = assert!(
    EventFrame::LEN == 8 + 1 + Path::LEN + 8 + TaskId::WIDTH,
    "事件那一段的宽度：号 ＋ kind ＋ 路 ＋ 号 ＋ 主人"
);

impl Message for EventFrame {
    type In = Event;
    type Buf = [u8; EventFrame::LEN];
    const EMPTY: Self::Buf = [0u8; EventFrame::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// 解开一条：**`kind` 不认识 ⇒ `None`**（表外那一条读不懂，与短一字节同一处置）
    fn fetch(bytes: &[u8]) -> Option<Event> {
        let (frame, _) = EventFrame::fetch_at(bytes, 0)?;
        Some(Event {
            seq: frame.seq,
            kind: Kind::of(frame.kind)?,
            road: frame.road,
            id: frame.id,
            owner: frame.owner,
        })
    }
}

/// **事件就是那个元素**（`Message` 的两只手直接走 `EventFrame`）：递出去的就是它的字节
/// （持树者那一具架的格子里装的正是它），于是 `Event` 这个类型自己就说清了"过边界的是什么"。
impl Message for Event {
    type In = Event;
    type Buf = [u8; EventFrame::LEN];
    const EMPTY: Self::Buf = [0u8; EventFrame::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        let at = self.seq.store_at(out, 0)?;
        let at = self.kind.code().store_at(out, at)?;
        let at = self.road.store_at(out, at)?;
        let at = self.id.store_at(out, at)?;
        self.owner.store_at(out, at)
    }

    fn fetch(bytes: &[u8]) -> Option<Event> {
        let (frame, _) = EventFrame::fetch_at(bytes, 0)?;
        Some(Event {
            seq: frame.seq,
            kind: Kind::of(frame.kind)?,
            road: frame.road,
            id: frame.id,
            owner: frame.owner,
        })
    }
}
