//! :frame 的提示之路（Tip）：GuestFrame（提示）· WiredFrame（线交接）·
//! `PlateFrame`（门牌）· 三枚码（`TIP_PLATE`/`TIP_GUEST`/`TIP_WIRED`）与入口 `TipIn`。

use env::{PieToken, TaskId};

use crate::common::path::{Path, PathBuf};

/// 提示之路上的两个 `kind`（首格；表外 ⇒ 这一帧读不懂）。
const TIP_PLATE: u8 = 1;

const TIP_GUEST: u8 = 2;

const TIP_WIRED: u8 = 3;

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct GuestFrame {
    pub kind: u8,
    pub who: TaskId,
}

/// 「门禁接线」那一形：**只有一个字节**（`kind`）——一句话，不带号。
/// **它说的是什么**：装配者已经把**名册**认下来了（补绑它自己与树），从那以后持树者那道门
/// **问得动身份**（operator::door::may）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WiredFrame {
    pub kind: u8,
}

/// # 为什么只有两格，且没有"填一枚 `Permit`"这一路
/// 这条路上装的是**装配者**（它请持树者替它落格）。装配者**报不出任何号**——它没有名录面
/// （`Roster` 只有 `bind` / `adopt`），也没有读格的那几手（`Tree` 只有"递上去"）⇒ 一枚
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rule {
    /// **不记许可**：与 `/svc/sys/operator/{…}` 那七格同一条口径——任何已绑身份都取得回。
    None,
    Root,
}

impl Rule {
    pub const WIDTH: usize = 1;
}

/// `Plate` 那一句：首格 `kind` ＋ 一条路 ＋ 末段那一枚 ＋ 规矩一格。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct PlateFrame {
    pub kind: u8,
    pub road: PathBuf,
    pub leaf: PieToken,
    pub rule: Rule,
}

const _: () = assert!(PlateFrame::LEN == 1 + Path::LEN + PieToken::WIDTH + Rule::WIDTH);

/// 提示之路上**最长那一形**的宽度（立一条路：PlateFrame）——两侧各备一只这么大的缓冲，
/// 收的那一侧按它拉。
pub const TIP_LEN: usize = PlateFrame::LEN;

/// **装配者推给持树者的一句话**（提示之路那一帧）。
/// 三形，各自的正文在变体上；共用的两句话：
///   会话），而树没有那条会话——它的生我者（编排域）是**替每一位客人转授**的那一侧，替不了
///   自己（自指 ⇒ 环）。树手里本来就握着**核**（Operator::land / `part`）
///   ⇒ "装配者递东西、持树者自己落"。
/// - **名字随帧来**：持树者不认识任何一族的名字（control::frame::DIR / `NAME` 都是递帧那一侧
///   的事实），它只答"把这一条路立出来"。
pub enum Tip {
    /// **在树上立一条路**：前缀逐段立成窗格（缺的就地造），末段按 `leaf` 落叶子或立窗格。
    /// 路是**绝对坐标**（从根起数），故 `/svc/sys/control`、`/svc/sys/operator`、`/svc/sys/operator/part`
    /// 三种落法**同一个形状**说得出来；再深一层、或"父底下立一块窗格"也不需要新格
    Plate {
        road: PathBuf,
        leaf: PieToken,
        rule: Rule,
    },
    /// **这一位是客人**。
    Guest(TaskId),
    /// **门禁接线**（装配者已认下名册）：一句话，不带号。
    Wired,
}

impl Tip {
    /// 编进 `out`，返写完的游标；装不下 / **路空** ⇒ `None`（路本身合法由 Path 保证）。
    pub fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Tip::Plate { road, leaf, rule } => {
                if road.is_empty() {
                    return None;
                }
                PlateFrame {
                    kind: TIP_PLATE,
                    road: road.clone(),
                    leaf: *leaf,
                    rule: *rule,
                }
                .store_at(out, 0)
            }
            Tip::Guest(who) => GuestFrame {
                kind: TIP_GUEST,
                who: *who,
            }
            .store_at(out, 0),
            Tip::Wired => WiredFrame { kind: TIP_WIRED }.store_at(out, 0),
        }
    }
}

/// **解开的一句**：路已经收进自己那一份（Path）。
pub enum TipIn {
    /// 立一条路（前缀逐段立窗格，末段按 `leaf`），并按 `rule` 决定要不要带一句规矩。
    Plate {
        road: PathBuf,
        leaf: PieToken,
        rule: Rule,
    },
    /// 这一位是客人。
    Guest(TaskId),
    /// 门禁接线。
    Wired,
}

impl TipIn {
    /// 解开一句：**首格 `kind` 决定形状**，长度必须是那一形该有的长度。
    /// 读不懂（表外的 `kind` / 路空 / 段数越界 / 长度不对）⇒ `None`：持树者据此报一行读数
    /// ——这条路上没有答话那一格，**别静默丢**。
    pub fn fetch(bytes: &[u8]) -> Option<TipIn> {
        match *bytes.first()? {
            TIP_PLATE => {
                let (frame, at) = PlateFrame::fetch_at(bytes, 0)?;
                // （`1 ＋ 1 ＋ 段数 × 32 ＋ 8 ＋ 1`：长短都不认）。两句都是本族的，derive 不替它判。
                if frame.road.is_empty() || at != bytes.len() {
                    return None;
                }
                Some(TipIn::Plate {
                    road: frame.road,
                    leaf: frame.leaf,
                    rule: frame.rule,
                })
            }
            TIP_GUEST if bytes.len() == GuestFrame::LEN => {
                Some(TipIn::Guest(GuestFrame::fetch(bytes)?.who))
            }
            TIP_WIRED if bytes.len() == WiredFrame::LEN => Some(TipIn::Wired),
            _ => None,
        }
    }
}

// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。
