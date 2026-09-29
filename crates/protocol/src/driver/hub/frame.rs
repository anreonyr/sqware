//! hub::frame — **形与码**：三面各一形，加一张失败域与状态码的双射表。
//!
//! ```text
//!   bond   面   [码 1B][类 32B][回信 8B]                 → [状态 1B]
//!   list   面   [码 1B][类 32B][游标 4B][回信 8B]         → [状态 1B][游标 4B][条数 1B][位 8B][名字 32B × n]
//!   claim  面   [码 1B][种 1B][取用 4B][形态 4B][主人 8B][回信 8B] → [状态 1B][名字 32B][线号 4B][号 8B]
//! ```
//!
//! **一原语一面**：三条问各有自己的门（`bond` / `list` 挂在 `/svc/hub` 下，`claim` 挂在**每台
//! 设备那一格**上），故**没有"一答多形"的 `Union`**——每一面的答各是一个定形。这与 operator
//! 那一族正相反：那边七条原语共用一扇门，答话才要四种形状。
//!
//! **回信孔是运输那一格，不排第一**（同 control 那条照实记）：它是"我给你的那一枚在你表里是
//! 几号"，对端据此一次 `Reserve` 就认得出，故不许被当作第二个名字使——三条问里它都排最后。
//!
//! **两处失败分得开**（同 [`crate::frame::Query`] 那条口径）：**长度 / 形状不对** ⇒ 这一帧读不懂；
//! **动作码不认得** ⇒ 也是读不懂。本族三形长度不同，故不认得的码连"往哪回"那一格都取不到
//! ⇒ [`Wire::take`] 一律答 `None`、不回话（control 那一族只有一形，它能答 `BAD`；**这一处差别
//! 是照实记，不是欠账**）。
//!
//! **`Deed` 里没有坐标**（`Key`）：区是 hub 自己的账据（它要与树对位），驱动拿到的是"哪一枚门闩
//! ＋ 哪条线 ＋ 它叫什么"——驱动这一侧一处都不用坐标。

use alloc::string::String;
use env::{Pair, PieToken};

use crate::message::Message;
use crate::system::operator::path::Path;

// ── 三个动作码 ──────────────────────────────────────────────

/// 报名：许我驱这一类。
pub const BOND: u8 = 1;
/// 列册：这一类里现在有哪几台、哪几台有主。
pub const LIST: u8 = 2;
/// 认领：这台归我。
pub const CLAIM: u8 = 3;

/// 一窗最多几条（取窗宽度）。**它是个旋钮，不是契约**——把 `Buf` 撑大就调它。
pub const LIST_MAX: usize = 4;

/// 回信孔那一枚上的记号。**三条问共用**：回信孔是每一趟自带的，与面无关。
pub const BACK_MARK: env::Mark = env::Mark::of("hub-back");

/// **报活孔**那一枚上的记号：主人（认领那一台的那位）铸一枚、**交一份给 hub**、此后一直开着。
///
/// hub 扫账时按它问"主人还在不在"（`mail::reserve`——与线路由者那条探活同一手）。内核那一问
/// （`UnitCall::Join`）只许**同队或父域**，而 hub 与驱动是**兄弟** ⇒ 主人那一枚只能由主人
/// 自己交过来（那条判据的实测见 `Claim.sensor` 那一格的头注）。
pub const ALIVE_MARK: env::Mark = env::Mark::of("hub-alive");

// ── 设备那一轴在树上的坐标（hub 落、驱动按它找）──────────────────

/// **设备那一轴在树上的路**：`/dev`（`/dev/<类>/<名>` 的头一段）。
///
/// **照实记（它为什么不与 `/svc` 同一层）**：`/svc` 底下是**常驻的东西**（各族服务 ＋ 驱动 ＋
/// 设备账那一台），而 `/dev` 底下是**设备那一本账**（hub 按机器自述落的格）——两者一件件对不上
/// （一台设备不对应一个域），故各占一层。
pub const DEV_ROAD: &Path = Path::new("dev");

/// **boot 那一类**：引导期那两件不按 `compatible` 认的东西（设备树本体 / 门铃）落在它底下
/// （`/dev/boot/{dtb,irq}`）——它们与设备同一条账（认领读法一模一样），只是"类"不是树里给的。
pub const BOOT: &str = "boot";

/// boot 那一类底下那两格的名字：**设备树本体**（hub 自己也要用它读名 / 类 / 线，
/// 但它同时是**树那一侧的客户**（`router` 要读 `riscv,ndev` 与 `interrupts-extended`））。
pub const DTB: &str = "dtb";

/// boot 那一类底下那两格的名字：**门铃**（中断那枚空载荷信号）。
pub const IRQ: &str = "irq";

// ── 失败域与状态码（一处编）────────────────────────────────

pub use crate::fail_codes::OK;

/// 没这件 / 这一类不在册上（这台机器没有这一类——是事实，不是错误）。
pub const UNKNOWN: u8 = 1;
/// 有人了（活着的不是我的主人）⇒ 换一台，或等它空出来。
pub const TAKEN: u8 = 2;
/// 授不出（门闩那一手没成）⇒ 装配错。
pub const DENIED: u8 = 3;
/// 那一枚孔用不动（对端没了 / 这一趟的路断了）⇒ 收摊。
///
/// **这一格是"本端判的"那一类**（同下面的 [`BAD`]）：hub 那一侧**从不答它**（它答得了的是
/// [`UNKNOWN`] / [`TAKEN`] / [`DENIED`] 三格）；答它的是**客侧那一手**——`port::ship` 或
/// `mail::push` / `recv` 说"这一枚用不动了"时，折的就是这一格（分开它的是**读数**：另一格
/// 是"这一趟没走到 / 读不懂"，下一步不同——[`Fail::Dead`] 是"收摊"，[`Fail::Bad`] 是
/// "这一趟别指望了"）。
pub const DEAD: u8 = 4;
/// **这一帧读不懂**（长短不对 / 形状不对）。不是对端说的事，是本端判的。
pub const BAD: u8 = 5;

/// 四格 ＋ 一格"读不懂"。**前四格对应四个不同的下一步**；[`Fail::Bad`] 是本端那一格。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 没这件 / 这一类不在册。
    Unknown,
    /// 有活着的主人。
    Taken,
    /// 授不出。
    Denied,
    /// 那一枚孔用不动（**本端判的**：这一枚的资源没了 / 权限不够 / 已交出去）。
    Dead,
    /// 这一趟没走到 / 读不懂（**本端判的**）。
    Bad,
}

crate::fail_codes! {
    /// 失败域 → 状态码（一处编：客侧与 hub 看同一张表）。
    ///
    /// **五格是双射**（含 [`Fail::Bad`]）：表外那一格由客侧那一手折成 [`Fail::Bad`]
    /// （同 control 的 `read`）。
    ///
    /// **照实记（后两格是"本端判的"那一类）**：hub 那一侧答得出的是前三格
    /// （[`UNKNOWN`] / [`TAKEN`] / [`DENIED`]）；[`Fail::Dead`] 与 [`Fail::Bad`] 由**客侧那一手**
    /// 分别在两处折出来（见 [`super::client`] 的 `call` / `read`）——它们照样在这张双射表里，
    /// 因为"客侧认哪一格"与"对端说哪一格"用的是同一张表（两个 `Fail` 值都是本族那一个）。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::Taken => TAKEN,
    Fail::Denied => DENIED,
    Fail::Dead => DEAD,
    Fail::Bad => BAD,
}

// ── 三条问 ──────────────────────────────────────────────────

/// 报名：**只有类**（驱动不需要知道盟号）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 41)]
pub struct Bond {
    pub op: u8,
    pub class: String,
    pub back: PieToken,
}

impl Bond {
    /// 编一问（动作码固定 [`BOND`]）。
    pub fn of(class: String, back: PieToken) -> Bond {
        Bond {
            op: BOND,
            class,
            back,
        }
    }
}

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

/// 列册：类 ＋ **游标**（从哪一条起取窗）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 45)]
pub struct ListReq {
    pub op: u8,
    pub class: String,
    pub from: u32,
    pub back: PieToken,
}

impl ListReq {
    /// 编一问（动作码固定 [`LIST`]）。
    pub fn of(class: String, from: u32, back: PieToken) -> ListReq {
        ListReq {
            op: LIST,
            class,
            from,
            back,
        }
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

/// 认领：**要什么权**（种 / 取用 / 形态）＋ **主人那一枚**。**"哪一台"不在这帧里**——你 `find`
/// 的是哪一格，那一格上挂的就是哪一台那一份孔。
///
/// **`sensor` 那一格为什么在这里**（照实记，量出来的）：hub 要判"这台的主人还在不在"，而
/// 内核那一问（`UnitCall::Join`）只许**同队或父域**（`kernel/src/runtime/switcher/envcall/unit.rs`
/// 的那条判据）——hub 与驱动是**兄弟**，问它答 `Denied`。故这条路上"主人那一枚"只能由主人自己
/// 交过来：驱动铸一枚只用来报活的孔，交一份给 hub，此后一直开着；hub 扫账时问它
/// （`mail::reserve`，与线路由者那条探活同一手）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Claim {
    pub op: u8,
    pub kind: u8,
    pub access: u32,
    pub policy: u32,
    pub sensor: PieToken,
    pub back: PieToken,
}

impl Claim {
    /// 编一问（动作码固定 [`CLAIM`]）。
    pub fn of(kind: u8, access: u32, policy: u32, sensor: PieToken, back: PieToken) -> Claim {
        Claim {
            op: CLAIM,
            kind,
            access,
            policy,
            sensor,
            back,
        }
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
    ///
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

// ── 入册那一趟（只走一次：装配者 → 本域）──────────────────────
//
// 它**不是一面**：没有动作码、没有客侧那一套（"面"是能面朝客人的权柄边界，而这一段只在起手
// 那一次出现）。可它必须说得出形状——条数就在帧里，因为**收的那一侧数不出"还有没有下一条"**。

// **照实记（"入册那条通道叫什么"这一格从本族退场了）**：它曾经在这里（`= super::NAME`），
// 而**唯一的读者是那一位自己的起手**——`programs/src/system/hub/server.rs` 铸孔时用的名字
// 取自**装配声明**（`programs/src/system/hub/program.rs` 的 `CHANNEL`：那一份只许引 `env`，
// 故那个字面量只能写在那儿）。两处同一个词、协议这一侧一处没人读 ⇒ 机制（名字）在一处就够，
// 死格退掉；"两处同一个词"那条照实记跟着搬到声明那一格上（见那儿的注）。

/// 一段入册最多几台。**它是个旋钮，不是契约**——上界对着**内核那一侧的配对块**：
/// `kernel/src/platform/devices.rs::MAX_PAIRS = 64`（那里一张门闩一行，装不下更多）；
/// 取同一个数 ⇒ "装不下"这件事在装配者那一步就现形（不是到了 hub 手里才发现少了几台）。
pub const ENROLL_MAX: usize = 64;

/// 这一段的定长缓冲：**最长那一形**（条数那一格 ＋ 上界那么多条记录，同一张表求和）。
pub const ENROLL_CAP: usize = Enroll::LEN;

/// **入册那一段**：条数 ＋ 那几条记录（[`Pair`] = 坐标 ＋ 那枚门闩**在收方表里**的号）。
///
/// **为什么是 `Pair` 而不是本族自己那一形**：装配者手里拿到的就是它——引导域按坐标授出、
/// 回一张 `Pair` 记录（见 `protocol::system::supply`），本段一个字节都不用翻译。
///
/// 段尾那一条恒是"设备树本体"：hub 要先把树读一遍才知道**哪一条是哪一台**（名 / 类 / 线），
/// 故装配者**先**领树、**先**把它编进这一段（次序即契约，见 `hub/server.rs` 的起手）。
///
/// **照实记（条数那一格是线上那一格）**：它从前分成两处——内存里 `n: usize`、线上另立一枚
/// `EnrollHead { n: u8 }`；derive 认"一段重复"之后合成一格，那个数在内存与线上是**同一个**。
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
    ///
    /// **"够长即可"**（多出来的那几字节不算读不懂）：这条判据是**本族的**，derive 不替它判。
    fn fetch(bytes: &[u8]) -> Option<Enroll> {
        let (enroll, _) = Enroll::fetch_at(bytes, 0)?;
        Some(enroll)
    }
}

// ── 三条答 ──────────────────────────────────────────────────

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
///
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
///
/// **两条一次说清**：`names[..n]` 是这一窗真正答出来的那些；`held` 的第 `i` 位对应 `names[i]`
/// （`1` = 这一台此刻有主）。
///
/// **照实记（这一格栽过：满窗那一版把空位也写出去）**：从前编解那两手是把 [`LIST_MAX`] 段
/// **全写**（空位写**旧那一枚定宽名字**的空值，即一整块零字节），而它读回来判非空 —— 于是**读回来
/// 当场解不动**（`n < LIST_MAX` 的每一窗都是这样）。症状是**客侧 `Unread`**、而服务端那一边
/// 一路"发得好好的"（读数：`hub: list sent n=1 err=false` ＋ `hub-c: recv unread`）。**修法**
/// 是"尾巴的长度由条数说、空位不上线"——今天这一句就是下面那条属性，由 `#[derive(env::Frame)]`
/// **一处**生成。
///
/// **照实记（这一形为什么曾经是"头 ＋ 手写尾巴"）**：这一格的"重复"从前不归 `#[derive]`，
/// 故另立了一枚 `WindowHead` 专门给尾巴算偏移、`store` / `fetch` 手写；derive 认"一段重复"
/// 之后那一枚并回本表（偏移与长度一处求和）。
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
    ///
    /// **照实记（这一格今天没有消费者）**：`held` 那一位掩码**就在帧里**（它是"这一台有没有
    /// 主人"那一问的答案），而现成的三位客人只取名字那一格（它们要的是"我有哪一台"）。
    /// 读它的是**发现那一面**（DSH 那种"现在谁占了什么"），今天还没有那位客人 ⇒ 这一格与
    /// 其它"机制在、客人不在"的格子同类：留在这里，是因为**去掉它掩码就成了读不出来的字段**。
    pub fn held(&self, i: usize) -> bool {
        self.held
            .get(i / 8)
            .is_some_and(|b| (b >> (i % 8)) & 1 == 1)
    }
}

/// **这一窗的缓冲那一格**：**最长那一形**（头 ＋ 满窗的名字，同一张表求和）。
///
/// 它是 `Buf` 的容量（收的那一侧备一只满的，装得下最宽那一窗），**不是线格式的长度**。
pub const WINDOW_LEN: usize = Window::LEN;

impl Message for Window {
    type In = Window;
    type Buf = [u8; WINDOW_LEN];
    const EMPTY: Self::Buf = [0u8; WINDOW_LEN];

    /// **尾巴只写前 `n` 段**（"条数即长度"）：那一句现在是属性，见 [`Window`] 的照实记。
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
