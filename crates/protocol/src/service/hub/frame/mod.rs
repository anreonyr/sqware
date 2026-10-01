//! hub::frame — **形与码**：三面各一形，加一张失败域与状态码的双射表。
//! ```text
//!   bond   面   [码 1B][类 32B][回信 8B]                 → [状态 1B]
//!   list   面   [码 1B][类 32B][游标 4B][回信 8B]         → [状态 1B][游标 4B][条数 1B][位 8B][名字 32B × n]
//!   claim  面   [码 1B][种 1B][取用 4B][形态 4B][主人 8B][回信 8B] → [状态 1B][名字 32B][线号 4B][号 8B]
//! ```
//! **一原语一面**：三条问各有自己的门（`bond` / `list` 挂在 `/svc/hub` 下，`claim` 挂在**每台
//! 设备那一格**上），故**没有"一答多形"的 `Union`**——每一面的答各是一个定形。这与 operator
//! 那一族正相反：那边七条原语共用一扇门，答话才要四种形状。

use alloc::string::String;
use env::{Pair, PieToken};

use crate::wire::message::Message;

pub use crate::wire::fail_codes::OK;


pub mod bond;
pub mod claim;
pub mod list;
pub mod vocab;

pub use self::bond::*;
pub use self::claim::*;
pub use self::list::*;
pub use self::vocab::*;
impl Message for Bond {
    type In = Bond;
    type Buf = [u8; Bond::LEN];
    const EMPTY: Self::Buf = [0u8; Bond::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// **恰好**（按游标判：名字变长，帧长不再等于 [`Bond::LEN`]——那是上界）且动作码是 [`BOND`]。
    fn fetch(bytes: &[u8]) -> Option<Bond> {
        let (q, at) = Bond::fetch_at(bytes, 0)?;
        (at == bytes.len() && q.op == BOND).then_some(q)
    }
}

impl Message for ListReq {
    type In = ListReq;
    type Buf = [u8; ListReq::LEN];
    const EMPTY: Self::Buf = [0u8; ListReq::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// **恰好**（按游标判：`class` 变长）。
    fn fetch(bytes: &[u8]) -> Option<ListReq> {
        let (q, at) = ListReq::fetch_at(bytes, 0)?;
        (at == bytes.len() && q.op == LIST).then_some(q)
    }
}

impl Message for Claim {
    type In = Claim;
    type Buf = [u8; Claim::LEN];
    const EMPTY: Self::Buf = [0u8; Claim::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Claim> {
        if bytes.len() != Claim::LEN {
            return None;
        }
        let q = Claim::fetch(bytes)?;
        (q.op == CLAIM).then_some(q)
    }
}

/// **收进来的一问**（与 control 那一族的 `Wire` 同形）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Wire {
    Bond(String),
    List(String, u32),
    Claim {
        kind: u8,
        access: u32,
        policy: u32,
        sensor: PieToken,
    },
}

impl Wire {
    /// 解一问：`(读出来的动作, 回信孔那一格)`。
    /// **不认得的码 ⇒ `None`**（不回话）：三形长度不同，读不出"往哪回"那一格——不猜。
    pub fn take(bytes: &[u8]) -> Option<(Option<Wire>, PieToken)> {
        match bytes.first().copied()? {
            BOND => {
                let q = Bond::fetch(bytes)?;
                Some((Some(Wire::Bond(q.class)), q.back))
            }
            LIST => {
                let q = ListReq::fetch(bytes)?;
                Some((Some(Wire::List(q.class, q.from)), q.back))
            }
            CLAIM => {
                let q = Claim::fetch(bytes)?;
                Some((
                    Some(Wire::Claim {
                        kind: q.kind,
                        access: q.access,
                        policy: q.policy,
                        sensor: q.sensor,
                    }),
                    q.back,
                ))
            }
            _ => None,
        }
    }
}

// 它**不是一面**：没有动作码、没有客侧那一套（"面"是能面朝客人的权柄边界，而这一段只在起手
// 那一次出现）。可它必须说得出形状——条数就在帧里，因为**收的那一侧数不出"还有没有下一条"**。

/// 一段入册最多几台。**它是个旋钮，不是契约**——上界对着**内核那一侧的配对块**：
/// `kernel/src/platform/devices.rs::MAX_PAIRS = 64`（那里一张门闩一行，装不下更多）；
/// 取同一个数 ⇒ "装不下"这件事在装配者那一步就现形（不是到了 hub 手里才发现少了几台）。
pub const ENROLL_MAX: usize = 64;

/// 这一段的定长缓冲：**最长那一形**（条数那一格 ＋ 上界那么多条记录，同一张表求和）。
pub const ENROLL_CAP: usize = Enroll::LEN;

/// **入册那一段**：条数 ＋ 那几条记录（[`Pair`] = 坐标 ＋ 那枚门闩**在收方表里**的号）。
/// **为什么是 `Pair` 而不是本族自己那一形**：装配者手里拿到的就是它——它按坐标从自己那本账
/// 取源、授出一枚、当场记一条 `Pair`（见 `programs/src/system/control/enroll.rs` 的
/// `enroll`），本段一个字节都不用翻译。
/// **这一段里必有"设备树本体"那一条**：hub 要先把树读一遍才知道**哪一条是哪一台**（名 / 类 / 线），
/// 故装配者一并把它授出（本族起手按坐标取它，见 `hub/serve/mod.rs`）。
#[derive(env::Frame, Clone, Copy)]
pub struct Enroll {
    n: u8,
    #[frame(count = n, fill = Pair::NONE)]
    records: [Pair; ENROLL_MAX],
}

/// **线上一个字节都不许动**：这一段那一形照上一句钉住（`[条数 1B][记录 40B × n]`）。
const _: () = assert!(Enroll::LEN == 1 + env::PAIR_LEN * ENROLL_MAX);

impl Enroll {
    /// 编一段入册。**条数越界 ⇒ `None`**（调用方按本地失败处置——那是"这台机器比内核那一侧
    /// 的配对块还大"，不是一条线上失败）。
    pub fn of(records: &[Pair]) -> Option<Enroll> {
        let n = records.len();
        if n > ENROLL_MAX {
            return None;
        }
        let mut held = [Pair::NONE; ENROLL_MAX];
        held.get_mut(..n)?.copy_from_slice(records);
        Some(Enroll {
            n: n as u8,
            records: held,
        })
    }

    /// 几条。
    pub fn len(&self) -> usize {
        self.n as usize
    }

    /// 这一段的第 `i` 条（越界 ⇒ `None`）。
    pub fn record(&self, i: usize) -> Option<Pair> {
        (i < self.len()).then(|| self.records[i])
    }
}

impl Message for Enroll {
    /// **写法与读法是同一个**：这一形只有一份。
    type In = Enroll;
    type Buf = [u8; ENROLL_CAP];
    const EMPTY: Self::Buf = [0u8; ENROLL_CAP];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// 解一段：**条数越界 / 不够长 ⇒ `None`**（不猜、不崩）。
    /// **"够长即可"**（多出来的那几字节不算读不懂）：这条判据是**本族的**，derive 不替它判。
    fn fetch(bytes: &[u8]) -> Option<Enroll> {
        let (enroll, _) = Enroll::fetch_at(bytes, 0)?;
        Some(enroll)
    }
}

/// 报名的答：只有状态那一格。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    pub status: u8,
}

impl Said {
    /// 编一答。
    pub const fn of(status: u8) -> Said {
        Said { status }
    }
}

impl Message for Said {
    type In = Said;
    type Buf = [u8; Said::LEN];
    const EMPTY: Self::Buf = [0u8; Said::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// 恰好 [`Said::LEN`]（长短都不认）。
    fn fetch(bytes: &[u8]) -> Option<Said> {
        if bytes.len() != Said::LEN {
            return None;
        }
        Said::fetch(bytes)
    }
}

/// **契**：认领那一答。**是扁平的一形**（状态 ＋ 名字 ＋ 线号 ＋ 号）——本仓的答话都不嵌套
/// 另一枚帧（`Field` 只管一格多宽，不认复合），故"契"与"状态"同住这一枚。
/// 三格各有各的消费者：`token` → `Device::open`（那一页在这一域表里是几号）；`line` →
/// 报给线路由者（**区→线那条权威在 hub**）；`name` → 本域那行读数。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 45)]
pub struct Deed {
    pub status: u8,
    pub name: String,
    pub line: u32,
    pub token: PieToken,
}

impl Deed {
    /// 空的一张：失败时那一答用它（`token` 是 [`PieToken::NONE`]）。
    pub const NONE: Deed = Deed {
        status: 0,
        name: String::new(),
        line: 0,
        token: PieToken::NONE,
    };

    /// 编一答：只有状态那一格（失败，或读不懂）。
    pub const fn of(status: u8) -> Deed {
        Deed {
            status,
            name: String::new(),
            line: 0,
            token: PieToken::NONE,
        }
    }

    /// 编一答：成了 ＋ 那张契。
    pub const fn granted(name: String, line: u32, token: PieToken) -> Deed {
        Deed {
            status: OK,
            name,
            line,
            token,
        }
    }
}

impl Message for Deed {
    type In = Deed;
    type Buf = [u8; Deed::LEN];
    const EMPTY: Self::Buf = [0u8; Deed::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// **恰好**（按游标判：`name` 变长；长短都不认）。
    fn fetch(bytes: &[u8]) -> Option<Deed> {
        let (q, at) = Deed::fetch_at(bytes, 0)?;
        (at == bytes.len()).then_some(q)
    }
}

/// **一窗**：状态 ＋ 游标 ＋ 条数 ＋ 有主那一位掩码 ＋ 至多 [`LIST_MAX`] 段名字。
/// **两条一次说清**：`names[..n]` 是这一窗真正答出来的那些；`held` 的第 `i` 位对应 `names[i]`
/// （`1` = 这一台此刻有主）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 142)]
pub struct Window {
    pub status: u8,
    pub from: u32,
    pub n: u8,
    pub held: [u8; 8],
    #[frame(count = n, fill = String::new())]
    pub names: [String; LIST_MAX],
}

/// **线上一个字节都不许动**：这一窗那一形照文件头那张表钉住
/// （`[状态 1B][游标 4B][条数 1B][位 8B][名字 32B × n]`）。属性换成手写也好、`MAX` 求和出错也好，
/// 这一句先红。
// 32 = 一枚名在**这一族**里的上界（长度那一字节 ＋ 至多 31 字节的内容）。

const _: () = assert!(Window::LEN == 1 + 4 + 1 + 8 + 32 * LIST_MAX);

impl Window {
    /// 空的一窗（失败，或这一类里一台都没有）。
    pub const EMPTY: Window = Window {
        status: OK,
        from: 0,
        n: 0,
        held: [0u8; 8],
        names: [const { String::new() }; LIST_MAX],
    };

    /// 第 `i` 条（`i < n` 才有）。
    pub fn name(&self, i: usize) -> Option<&String> {
        (i < self.n as usize).then(|| &self.names[i])
    }

    /// 第 `i` 条此刻有没有主。
    pub fn held(&self, i: usize) -> bool {
        self.held
            .get(i / 8)
            .is_some_and(|b| (b >> (i % 8)) & 1 == 1)
    }
}

/// **这一窗的缓冲那一格**：**最长那一形**（头 ＋ 满窗的名字，同一张表求和）。
/// 它是 `Buf` 的容量（收的那一侧备一只满的，装得下最宽那一窗），**不是线格式的长度**。
pub const WINDOW_LEN: usize = Window::LEN;

impl Message for Window {
    type In = Window;
    type Buf = [u8; WINDOW_LEN];
    const EMPTY: Self::Buf = [0u8; WINDOW_LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// **恰好** 头 ＋ `n` 段名字（条数越界由那一格自己拦；长度与条数对不上答 `None`——
    /// 这条判据是**本族的**，derive 不替它判）。
    fn fetch(bytes: &[u8]) -> Option<Window> {
        let (window, at) = Window::fetch_at(bytes, 0)?;
        (at == bytes.len()).then_some(window)
    }
}
