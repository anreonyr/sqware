//! 帧、码、记号

use alloc::string::String;
use env::PieToken;

use crate::common::path::PathBuf;

use crate::wire::id::Id as _;
use crate::wire::message::Message;

pub mod road;
pub mod tip;
pub mod vocab;
pub mod watch;

pub use self::road::*;
pub use self::tip::*;
pub use self::vocab::*;
pub use self::watch::{Event, Kind};
impl crate::wire::id::Id for EntryId {
    fn new(raw: usize) -> EntryId {
        EntryId::new(raw)
    }

    fn get(self) -> usize {
        EntryId::get(self)
    }
}

/// **号那一格线上是 8 字节小端**——与 crate::wire::id::Id 给三条号空间定的同一条规则（那一条 trait
impl env::wire::Field for EntryId {
    const WIDTH: usize = 8;
    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }
    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(Self::from_bytes(bytes.get(..8)?.try_into().ok()?))
    }
}

/// 成功那一格：**全协议同一个号**——定义在 crate::wire::OK，本族只把它转出来
/// （crate::WireCodes 派生的两向读法就是拿它当"没失败"那一格）
pub use crate::wire::OK;

use self::vocab::{FIND, LAND, LIST, NAME, PART, SEEK, TRIM, WATCH};

/// **解开的一问**（名字已经是 String，故不是借用）
/// 与 Req 是一对：编的时候按动作分形状，解的时候也按动作分形状——`op` 与荷载不配
/// （比如 `LAND` 那一码配上一枚号）解不出来，持树者据此答 BAD
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Wire {
    /// `seek`：一条路（段数与段都在 Path 里；超上限根本造不出来，故 FULL 不再来自它）
    Road(PathBuf),
    /// `list`：容器坐标
    List(Where),
    /// `part`：容器坐标 + 新名
    Part {
        at: Where,
        name: String,
    },
    Land {
        at: Where,
        name: String,
        entry: PieToken,
        permit: Permit,
        mine: bool,
    },
    /// `find` / `trim` / `name`：一枚号（三者的形状一样，故解出来仍是三格）
    Find(EntryId),
    Trim(EntryId),
    Name(EntryId),
    /// `watch`：订 `road` 这条子树；`page` / `bell` 是订阅者自己铸的那两块（页 ＋ 铃）
    Watch {
        road: PathBuf,
        page: PieToken,
        bell: PieToken,
    },
}

impl Message for Req {
    type In = Wire;
    type Buf = [u8; REQ_LEN];
    const EMPTY: Self::Buf = [0u8; REQ_LEN];

    /// 编进 `out`：**动作码由形状给**（不在别处再写一遍），偏移与长度由字段表求和
    /// `Road` 那一格的正文（路）也回表了（RoadFrame：动作码 ＋ 路）——偏移一处都不写
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Req::Road(road) => RoadFrame {
                op: SEEK,
                road: road.clone(),
            }
            .store_at(out, 0),
            Req::List(at) => List { op: LIST, at: *at }.store_at(out, 0),
            Req::Part { at, name } => Part {
                op: PART,
                at: *at,
                name: name.clone(),
            }
            .store_at(out, 0),
            Req::Land {
                at,
                name,
                entry,
                permit,
                mine,
            } => Land {
                op: LAND,
                at: *at,
                name: name.clone(),
                entry: *entry,
                mine: *mine,
                permit: *permit,
            }
            .store_at(out, 0),
            Req::Find(id) => Entry { op: FIND, id: *id }.store_at(out, 0),
            Req::Trim(id) => Entry { op: TRIM, id: *id }.store_at(out, 0),
            Req::Name(id) => Entry { op: NAME, id: *id }.store_at(out, 0),
            Req::Watch { road, page, bell } => WatchFrame {
                op: WATCH,
                road: road.clone(),
                page: *page,
                bell: *bell,
            }
            .store_at(out, 0),
        }
    }

    /// 解开一问：**`op` 决定形状**（见文件头那张表）。**读不懂返 `None`**（持树者据此答
    /// BAD）
    /// **长度为该形状该有的长度是帧的契约**（各张表的 `LEN`，`store` 产出的就是那个长度）
    /// 故短一字节、长一字节都读不懂
    fn fetch(bytes: &[u8]) -> Option<Wire> {
        let op = *bytes.first()?;
        Some(match op {
            // **长度即形状**：`1 ＋ 1 ＋ 段数 × 32`（段数那一格在 Path 里；条数与长度对不对
            // 由下面那一句判——短一字节、长一字节都答"读不懂"）。
            SEEK => {
                let (frame, at) = RoadFrame::fetch_at(bytes, 0)?;
                if at != bytes.len() {
                    return None;
                }
                Wire::Road(frame.road)
            }
            LIST if bytes.len() == List::LEN => Wire::List(List::fetch(bytes)?.at),
            // 这两形含一枚变长名字 ⇒ **"恰好"按游标判**（帧长不再等于那张表的 `LEN`）。
            PART => {
                let (frame, end) = Part::fetch_at(bytes, 0)?;
                if end != bytes.len() {
                    return None;
                }
                Wire::Part {
                    at: frame.at,
                    name: frame.name,
                }
            }
            LAND => {
                let (frame, end) = Land::fetch_at(bytes, 0)?;
                if end != bytes.len() {
                    return None;
                }
                Wire::Land {
                    at: frame.at,
                    name: frame.name,
                    entry: frame.entry,
                    permit: frame.permit,
                    mine: frame.mine,
                }
            }
            FIND | TRIM | NAME if bytes.len() == Entry::LEN => {
                let id = Entry::fetch(bytes)?.id;
                match op {
                    FIND => Wire::Find(id),
                    TRIM => Wire::Trim(id),
                    _ => Wire::Name(id),
                }
            }
            // `watch` 含一条变长路 ⇒ 与 `PART` / `LAND` 同一处置：**"恰好"按游标判**。
            WATCH => {
                let (frame, end) = WatchFrame::fetch_at(bytes, 0)?;
                if end != bytes.len() {
                    return None;
                }
                Wire::Watch {
                    road: frame.road,
                    page: frame.page,
                    bell: frame.bell,
                }
            }
            // 没见过的动作码、或长度不是这张形状该有的那个 ⇒ 读不懂（不另立一格）。
            _ => return None,
        })
    }
}

// 手写的那六手（`op_of` / `unpack_ask` / `unpack_at` / `unpack_id` / `unpack_name` / `tail`）与

impl env::wire::Field for Rule {
    const WIDTH: usize = Rule::WIDTH;

    fn store(&self, out: &mut [u8]) {
        out[0] = match *self {
            Rule::None => 0,
            Rule::Root => 1,
        };
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        match *bytes.first()? {
            0 => Some(Rule::None),
            1 => Some(Rule::Root),
            // 表外的记 ⇒ 整帧读不懂（同 `Permit` / `Where` 那一格的口径）。
            _ => None,
        }
    }
}
