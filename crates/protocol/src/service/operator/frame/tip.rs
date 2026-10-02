//! :frame 的提示之路（Tip）：GuestFrame（提示）· WiredFrame（线交接）·
//! `PlateFrame`（门牌）· 三枚码（`TIP_PLATE`/`TIP_GUEST`/`TIP_WIRED`）与入口 `TipIn`。

use env::{PieToken, TaskId};
use env::wire::Field;

use crate::common::path::{Path, PathBuf};
use super::{EntryId, Permit};

/// Bootstrap acknowledgements are separate from ordinary Operator sessions.
pub const TIP_BACK: env::Mark = env::Mark::of("operator-tip-back");

/// 提示之路上的两个 `kind`（首格；表外 ⇒ 这一帧读不懂）
const TIP_PLATE: u8 = 1;

const TIP_GUEST: u8 = 2;

const TIP_WIRED: u8 = 3;
const TIP_UNPLATE: u8 = 4;
const TIP_EMPTY: u8 = 5;
const TIP_ABORT: u8 = 6;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct GuestFrame {
    pub kind: u8,
    pub who: TaskId,
}

/// 门禁接线：可信装配者指定 authority 与三枚收件者表中的查询入口。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WiredFrame {
    pub kind: u8,
    pub authority: TaskId,
    pub resolve: PieToken,
    pub matches: PieToken,
    pub same: PieToken,
    pub back: PieToken,
}

/// `Plate` 那一句：首格 `kind` ＋ 一条路 ＋ 末段那一枚 ＋ 完整许可
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct PlateFrame {
    pub kind: u8,
    pub road: PathBuf,
    pub leaf: PieToken,
    pub permit: Permit,
    pub owner: TaskId,
    pub replace: u8,
    pub back: PieToken,
}

const _: () = assert!(PlateFrame::LEN == 2 + Path::LEN + TaskId::WIDTH + 2 * PieToken::WIDTH + <Permit as env::wire::Field>::WIDTH);

/// 提示之路上**最长那一形**的宽度（立一条路：PlateFrame）——两侧各备一只这么大的缓冲
/// 收的那一侧按它拉
pub const TIP_LEN: usize = PlateFrame::LEN;

/// **装配者推给持树者的一句话**（提示之路那一帧）
/// 三形，各自的正文在变体上；共用的两句话
/// 会话），而树没有那条会话——它的生我者（编排域）是**替每一位客人转授**的那一侧，替不了
/// 自己（自指 ⇒ 环）。树手里本来就握着**核**（Operator::land / `part`）
/// ⇒ "装配者递东西、持树者自己落"
/// - **名字随帧来**：持树者不认识任何一族的名字（control::frame::DIR / `NAME` 都是递帧那一侧
/// 的事实），它只答"把这一条路立出来"
pub enum Tip {
    /// **在树上立一条路**：前缀逐段立成窗格（缺的就地造），末段按 `leaf` 落叶子或立窗格
    /// 路是**绝对坐标**（从根起数），故 `/svc/sys/control`、`/svc/sys/operator`、`/svc/sys/operator/part`
    /// 三种落法**同一个形状**说得出来；再深一层、或"父底下立一块窗格"也不需要新格
    Plate {
        road: PathBuf,
        leaf: PieToken,
        permit: Permit,
        owner: TaskId,
        replace: bool,
        back: PieToken,
    },
    /// **这一位是客人**
    Guest(TaskId),
    Abort { road: PathBuf, leaf: PieToken, back: PieToken },
    Empty { road: PathBuf, back: PieToken },
    Unplate { id: EntryId, back: PieToken },
    /// 门禁接线：完整来源绑定的查询束。
    Wired { authority: TaskId, resolve: PieToken, matches: PieToken, same: PieToken, back: PieToken },
}

impl Tip {
    /// 编进 `out`，返写完的游标；装不下 / **路空** ⇒ `None`（路本身合法由 Path 保证）
    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Tip::Plate { road, leaf, permit, owner, replace, back } => {
                if road.is_empty() {
                    return None;
                }
                PlateFrame {
                    kind: TIP_PLATE,
                    road: road.clone(),
                    leaf: *leaf,
                    permit: *permit,
                    owner: *owner,
                    replace: u8::from(*replace),
                    back: *back,
                }
                .store_at(out, 0)
            }
            Tip::Abort { road, leaf, back } => AbortFrame { kind: TIP_ABORT, road: road.clone(), leaf: *leaf, back: *back }.store_at(out, 0),
            Tip::Empty { road, back } => EmptyFrame { kind: TIP_EMPTY, road: road.clone(), back: *back }.store_at(out, 0),
            Tip::Unplate { id, back } => {
                let mut at = 0;
                use env::wire::Field;
                *out.get_mut(at)? = TIP_UNPLATE; at += 1;
                id.store(out.get_mut(at..at + EntryId::WIDTH)?); at += EntryId::WIDTH;
                back.store(out.get_mut(at..at + PieToken::WIDTH)?); at += PieToken::WIDTH;
                Some(at)
            }
            Tip::Guest(who) => GuestFrame {
                kind: TIP_GUEST,
                who: *who,
            }
            .store_at(out, 0),
            Tip::Wired { authority, resolve, matches, same, back } => WiredFrame {
                kind: TIP_WIRED,
                authority: *authority,
                resolve: *resolve,
                matches: *matches,
                same: *same,
                back: *back,
            }.store_at(out, 0),
        }
    }
}

impl crate::wire::message::Message for Tip {
    type In = TipIn;
    type Buf = [u8; TIP_LEN];
    const EMPTY: Self::Buf = [0; TIP_LEN];

    fn store(&self, bytes: &mut [u8]) -> Option<usize> {
        Tip::store(self, bytes)
    }
    fn fetch(bytes: &[u8]) -> Option<Self::In> {
        TipIn::fetch(bytes)
    }
}

/// **解开的一句**：路已经收进自己那一份（Path）
pub enum TipIn {
    /// 立一条路，叶子携带显式许可。
    Plate {
        road: PathBuf,
        leaf: PieToken,
        permit: Permit,
        owner: TaskId,
        replace: bool,
        back: PieToken,
    },
    /// 这一位是客人
    Guest(TaskId),
    Abort { road: PathBuf, leaf: PieToken, back: PieToken },
    Empty { road: PathBuf, back: PieToken },
    Unplate { id: EntryId, back: PieToken },
    /// 门禁接线
    Wired { authority: TaskId, resolve: PieToken, matches: PieToken, same: PieToken, back: PieToken },
}

impl TipIn {
    /// 解开一句：**首格 `kind` 决定形状**，长度必须是那一形该有的长度
    /// 读不懂（表外的 `kind` / 路空 / 段数越界 / 长度不对）⇒ `None`：持树者据此报一行读数
    pub fn fetch(bytes: &[u8]) -> Option<TipIn> {
        match *bytes.first()? {
            TIP_PLATE => {
                let (frame, at) = PlateFrame::fetch_at(bytes, 0)?;
                // （`1 ＋ 1 ＋ 段数 × 32 ＋ 8 ＋ 1`：长短都不认）。两句都是本族的，derive 不替它判。
                if frame.road.is_empty() || at != bytes.len() || frame.replace > 1 {
                    return None;
                }
                Some(TipIn::Plate {
                    road: frame.road,
                    leaf: frame.leaf,
                    permit: frame.permit,
                    owner: frame.owner,
                    replace: frame.replace == 1,
                    back: frame.back,
                })
            }
            TIP_ABORT => {
                let (frame, at) = AbortFrame::fetch_at(bytes, 0)?;
                if at != bytes.len() || frame.road.is_empty() { return None; }
                Some(TipIn::Abort { road: frame.road, leaf: frame.leaf, back: frame.back })
            }
            TIP_EMPTY => {
                let (frame, at) = EmptyFrame::fetch_at(bytes, 0)?;
                if at != bytes.len() || frame.road.is_empty() { return None; }
                Some(TipIn::Empty { road: frame.road, back: frame.back })
            }
            TIP_UNPLATE if bytes.len() == 1 + EntryId::WIDTH + PieToken::WIDTH => {
                use env::wire::Field;
                Some(TipIn::Unplate { id: EntryId::fetch(&bytes[1..1 + EntryId::WIDTH])?,
                    back: PieToken::fetch(&bytes[1 + EntryId::WIDTH..])? })
            }
            TIP_GUEST if bytes.len() == GuestFrame::LEN => {
                Some(TipIn::Guest(GuestFrame::fetch(bytes)?.who))
            }
            TIP_WIRED if bytes.len() == WiredFrame::LEN => {
                let frame = WiredFrame::fetch(bytes)?;
                Some(TipIn::Wired {
                    authority: frame.authority,
                    resolve: frame.resolve,
                    matches: frame.matches,
                    same: frame.same,
                    back: frame.back,
                })
            }
            _ => None,
        }
    }
}

// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

#[derive(env::Frame, Clone, Debug, PartialEq, Eq)]
struct EmptyFrame { kind: u8, road: PathBuf, back: PieToken }

#[derive(env::Frame, Clone, Debug, PartialEq, Eq)]
struct AbortFrame { kind: u8, road: PathBuf, leaf: PieToken, back: PieToken }
