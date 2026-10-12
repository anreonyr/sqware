//! Operator 的可信 bootstrap 与查询接线提示。

use env::wire::Span as _;
use env::{PieToken, TaskId};

/// Bootstrap acknowledgements are separate from ordinary Operator sessions.
pub use crate::operator::marks::TIP_BACK;

/// 提示之路上的两个 `kind`（首格；表外 ⇒ 这一帧读不懂）
const TIP_GUEST: u8 = 2;

const TIP_WIRED: u8 = 3;
const TIP_ROOT: u8 = 7;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct GuestFrame {
    pub kind: u8,
    pub who: TaskId,
    pub reply: PieToken,
    pub ask: PieToken,
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

/// Largest bootstrap frame.
pub const TIP_LEN: usize = WiredFrame::LEN;

/// **装配者推给持树者的一句话**（提示之路那一帧）
/// 三形，各自的正文在变体上；共用的两句话
/// 会话），而树没有那条会话——它的生我者（编排域）是**替每一位客人转授**的那一侧，替不了
/// 自己（自指 ⇒ 环）。树手里本来就握着**核**（Operator::land / `part`）
/// ⇒ "装配者递东西、持树者自己落"
/// - **名字随帧来**：持树者不认识任何一族的名字（control::frame::DIR / `NAME` 都是递帧那一侧
/// 的事实），它只答"把这一条路立出来"
pub enum Tip {
    /// **这一位是客人**
    Guest {
        who: TaskId,
        reply: PieToken,
        ask: PieToken,
    },
    Root {
        back: PieToken,
    },
    /// 门禁接线：完整来源绑定的查询束。
    Wired {
        authority: TaskId,
        resolve: PieToken,
        matches: PieToken,
        same: PieToken,
        back: PieToken,
    },
}

impl Tip {
    /// 编进 `out`，返写完的游标；装不下 / **路空** ⇒ `None`（路本身合法由 Path 保证）
    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Tip::Guest { who, reply, ask } => GuestFrame {
                kind: TIP_GUEST,
                who: *who,
                reply: *reply,
                ask: *ask,
            }
            .store_at(out, 0),
            Tip::Root { back } => RootFrame {
                kind: TIP_ROOT,
                back: *back,
            }
            .store_at(out, 0),
            Tip::Wired {
                authority,
                resolve,
                matches,
                same,
                back,
            } => WiredFrame {
                kind: TIP_WIRED,
                authority: *authority,
                resolve: *resolve,
                matches: *matches,
                same: *same,
                back: *back,
            }
            .store_at(out, 0),
        }
    }
}

impl wire::message::Message for Tip {
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

/// Decoded bootstrap request.
pub enum TipIn {
    /// 这一位是客人
    Guest {
        who: TaskId,
        reply: PieToken,
        ask: PieToken,
    },
    Root {
        back: PieToken,
    },
    /// 门禁接线
    Wired {
        authority: TaskId,
        resolve: PieToken,
        matches: PieToken,
        same: PieToken,
        back: PieToken,
    },
}

impl TipIn {
    /// 解开一句：**首格 `kind` 决定形状**，长度必须是那一形该有的长度
    /// 读不懂（表外的 `kind` / 路空 / 段数越界 / 长度不对）⇒ `None`：持树者据此报一行读数
    pub fn fetch(bytes: &[u8]) -> Option<TipIn> {
        match *bytes.first()? {
            TIP_GUEST if bytes.len() == GuestFrame::LEN => {
                let frame = GuestFrame::fetch_at(bytes, 0)?.0;
                Some(TipIn::Guest {
                    who: frame.who,
                    reply: frame.reply,
                    ask: frame.ask,
                })
            }
            TIP_ROOT if bytes.len() == RootFrame::LEN => {
                let frame = RootFrame::fetch_at(bytes, 0)?.0;
                Some(TipIn::Root { back: frame.back })
            }
            TIP_WIRED if bytes.len() == WiredFrame::LEN => {
                let frame = WiredFrame::fetch_at(bytes, 0)?.0;
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

#[derive(env::Frame, Clone, Copy, Debug, PartialEq, Eq)]
struct RootFrame {
    kind: u8,
    back: PieToken,
}
