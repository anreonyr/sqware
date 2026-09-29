//! coalition 的**帧那一半** —— 帧与码（内核那一只手的别名在 `protocol` 那一侧的 `mod.rs`）。
//!
//! 本文件**不做裁决**：盟册的规矩全在实现侧那一本账里（`programs/src/system/coalition/core.rs`）。这里只有三件事——
//! 把失败域翻成答话码、把答案编进答话那一格、以及**本族**那几格码 / 记号 / **窗**那一档。
//!
//! **照实记（这一份为什么拆出来）**：见 `principal/frame.rs` 的同一条——帧形的边角机器走不到，
//! 拆开是为了让它们在**宿主靶**上编得动；**那台靶已删**（用户裁定"protocol-case 没必要"）⇒
//! 这一份照旧只认 `env` 与同层 `core`，但那些边角今天**没有判据**。
//!
//! # 帧（与 `system::principal::frame` 同一形状；窗那一档多一种答形）
//!
//! ```text
//!   Query  [0] op   [1..9] a   [9..17] b   [17..25] back     25
//!   Reply  [0] status  [1] flag  [2..10] a                   10（`crate::frame` 那一份）
//!          [0] status                                        1（失败那几格）
//!          [0] status  [1] 未完  [2] 条数  [3..] 号           3 + 8n，上界 [`UNION_LEN`] = 131
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`FOUND` 两格都空，`ENTER` / `LEAVE` 只用 `a`，
//! `AMID` 两格都用，`BAND` / `BLOC` 的 `b` 是**游标**）——而"这一条有几格"由下面的
//! [`Req`] / [`Wire`] 按类型说。答话的**一格答**那一形本体在 [`crate::frame`]（两族同形，故
//! 只有一份）；**格状态 ＋ 窗**那两档留在这里（只此一族），三形合成 [`Union`]——**形状由长度分**，
//! 故写法与读法是同一个。
//!
//! **照实记（这一行原写"一问 17 字节"）**：那是 `back` 那一格落地之前抄的，此后一问一直是
//! `1 + 8 + 8 + 8 = 25`（同 `principal/frame.rs` 那条，详见 [`crate::frame`]）。
//!
//! **游标是阈值，说在 `b` 那一格**：`b = 游标 + 1`，`0` = 没有游标（从头取）。加一是有理的
//! ——**零号是真格子**（`PrincipalId::ROOT` 是 0、`CoalitionId(0)` 是一枚普通的盟），
//! 拿 0 当"没有"会把那一位漏掉。取的是**号 > 阈值**的那些，故没有"过期游标"这回事。
//!
//! **报文里没有"我是谁"这一格**：发送者由内核在 `Push` 那一刻盖章，Server 拿去名册问。
//!
//! # 编答的助手**少一个**
//!
//! `system::principal::frame` 有 `reply_present`（"有没有一条号"），这里不需要——
//! 本族没有"可能没有的一条号"那种答案（`found` 必有号，`amid` 是是非）。**帮手少一个，
//! 是原语少一条的余数。**
//!
//! # 码的数字**不照抄 principal**
//!
//! 同一个概念 `UNKNOWN`，operator 那一面是 1、principal 那一面是 2——三家各按**自己失败域
//! 的顺序**排、`BAD` 收尾。故本族按自己的两格排（见 `fail_codes!` 那张表）：照抄别家只会
//! 让自己表里空出一个号。

use crate::id::Id;
use crate::message::Message;
use crate::system::principal::PrincipalId;
use env::{Mark, PieToken, TaskId};

use crate::system::operator::path::Path;

// ── 上线的类型（原先住 `core.rs`：残枝那一刀并进来）──────────────

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CoalitionId(usize);

impl CoalitionId {
    /// 由裸号造一个（线上解码面；没铸过的号从这里进来）。
    pub const fn new(raw: usize) -> CoalitionId {
        CoalitionId(raw)
    }

    /// 裸号。
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Id for CoalitionId {
    fn new(raw: usize) -> CoalitionId {
        CoalitionId::new(raw)
    }

    fn get(self) -> usize {
        CoalitionId::get(self)
    }
}

/// 这一枚号在线上是 **8 字节小端**——口径与 `operator::EntryId` 那一处相同（**impl 跟着类型走**，
/// `env` 不认识 [`CoalitionId`]）。读的那一侧**不校验"铸过没有"**：解出来的号在不在盟册里由
/// 核心答（`Fail::Unknown`）。
impl env::wire::Field for CoalitionId {
    const WIDTH: usize = 8;

    fn store(&self, out: &mut [u8]) {
        out.copy_from_slice(&self.to_bytes());
    }

    fn fetch(bytes: &[u8]) -> Option<Self> {
        Some(Self::from_bytes(bytes.get(..8)?.try_into().ok()?))
    }
}

// ── 失败域 ──────────────────────────────────────────────────

/// 失败域：**三格**，每格一个**不同的下一步**。
///
/// **照实记（第三格是开面那一刀添的）**：本族原先**两格、没有 `Denied`**——那时确实没有：
/// 盟无主，三条写里的门要么是"这条号是假的"，要么是"备不下"，故横向那条轴与纵向那条轴
/// （[`system::principal`](crate::system::principal) 有 `Denied`）在失败域上分得开。今天多出来的
/// [`Fail::Denied`] 问的是**另外一件事**：不是"你得请谁来做"，是"**你手里那一枚门牌给不给这一
/// 条**"（载体那一维，见 [`super::grant`]）。**核心那一侧的口径一字未动**——它照旧没有一处
/// "你得请谁来做"的判断。
///
/// **三条读里只有 `bloc` 没有失败域**：`amid` / `band` 问的是**本册自己的**号空间，故都会答
/// "查无此盟"；`bloc` 问的是**别人的**号空间——`p` 是别人给的标签，本册不去问身份服务，
/// 不在任何盟里就是空串。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 这枚盟不存在（从来没铸过），或这个 TID 没绑过。调用方要改的是：**我手里这个号是假的**
    /// 或**我还没有身份**。
    Unknown,
    /// `try_reserve` 备不下。调用方要改的是：**晚点再来**。
    ///
    /// **照实记（这一格的产出者从一条变成两条）**：K2 翻案之前只有 [`Coalition::enter`] 到得了
    /// 这一格（`found` 不分配、只动计数器）；翻案之后 `found` 要多记一行**盟主** ⇒ 它也到得了
    /// 这一格。`leave` 与三条读照旧不分配。
    Full,
    /// 你手里那一枚门牌给不了这一条：**换一枚**（或换一位客人），别重试。
    ///
    /// **照实记（这一格是开面那一刀加的；它从前没有）**：这一族本来的正文写着"两格，
    /// **没有 `Denied`**：盟无主，没有一处'你得请谁来做'的判断"——那是对**核心**说的，而它
    /// 今天仍然成立（核心那三条写原语的钥匙是"你是不是一条已绑身份"）。这一格问的是**载体**
    /// 的事：**会话说的是哪一枚门牌**（[`Grant`](super::grant::Grant)：问面 / 定面），而"哪一枚"
    /// 与"你是谁"是两件事。加面之前它一个字都用不上；加面之后它是**判面那一句的出口**。
    Denied,
    /// **你不是这一枚盟的盟主** ⇒ 别拿它来代报名（要改的是"换一条路"，不是"再试一次"）。
    ///
    /// **照实记（这一格是 K2 翻案那一刀加的）**：这一族本来的裁定是"**盟无主**"——立盟那位
    /// 不留名，故谁都不比谁大，"替别人入盟"这件事在模型里说不出来。翻案翻的就是**这一格不存在**：
    /// 设备账那一台要替四位驱动报名（`hub::bond` ⇒ `admit`），而"入"的钥匙是**发送者那一格**
    /// （`Coalition::enter` 只表达"**我**进这枚盟"）⇒ 没有主就没有一条路说得通。
    ///
    /// **翻的是哪一格、没翻哪一格**：盟籍那一格照旧**只是一对号**（没有角色、没有权重）；
    /// 多出来的是一格**盟主**（立盟那位），而它只答一句话——"代报名这件事，归不归你"。
    /// 于是 [`Coalition::enter`] 那条"诚实性由签名给"的口径**照旧成立**（它仍旧只做得了"我进"），
    /// 而"替别人进"从此有了**一条带名字的路**：盟主点名，服务端过名册把名点实。
    NotChief,
}

// ── 一窗号 ──────────────────────────────────────────────────

/// 一窗最多几枚号。条数是策略、容器要有界 ⇒ 窗口有顶，**"还有没有"由 `more` 说**。
pub const WINDOW_CAP: usize = 16;

/// 一窗号：**一趟读的读数**（最多 [`WINDOW_CAP`] 枚，**号序升序**）。
///
/// 空位是 `None` 而不是 `T::new(0)`：**零号是真格子**（`PrincipalId::ROOT` 就是 0），
/// 拿它当"这一格空着"正是要避开的那件事。
///
/// **与 operator 那个 [`Listing`](crate::system::operator::frame::Listing) 不合并**：那一边一条 pane
/// **有顶**，故没有"未完"这一格；本族靠 `more` 分页。两处各留一个的理由（连帧形那一半）
/// 写在那边。
///
/// **取窗落在核心**（[`Coalition::band`] / [`Coalition::bloc`] 扫一遍表就填出来）：服务那一层
/// 只把它编成帧，不做选择。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window<T: Id> {
    items: [Option<T>; WINDOW_CAP],
    n: usize,
    more: bool,
}

impl<T: Id> Window<T> {
    /// 空的那一串（`more = false`）。
    pub const fn new() -> Window<T> {
        Window {
            items: [None; WINDOW_CAP],
            n: 0,
            more: false,
        }
    }

    /// 由一串号凑一窗（`more` = 窗外还有）——**解码面**：线上收来的那一窗由这里成形。
    ///
    /// 收够 [`WINDOW_CAP`] 枚就停：帧长了是帧的毛病，读的人只认窗前这些（帧长与条数对不对
    /// 由 `protocol` 那一侧的 `frame` 那一层先挡掉）。
    pub fn gather(more: bool, ids: impl Iterator<Item = T>) -> Window<T> {
        let mut out = Window::new();
        for id in ids.take(WINDOW_CAP) {
            out.push(id);
        }
        out.more = more;
        out
    }

    /// 几枚。
    pub fn len(&self) -> usize {
        self.n
    }

    /// 窗外还有没有（这一趟没答完的那些）。
    pub fn more(&self) -> bool {
        self.more
    }

    /// 第 `at` 枚（号序；越界 ⇒ `None`）。
    pub fn get(&self, at: usize) -> Option<T> {
        if at < self.n {
            self.items.get(at).copied().flatten()
        } else {
            None
        }
    }

    /// 号序走一遍。
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.items[..self.n].iter().filter_map(|slot| *slot)
    }

    /// 末一枚——**它就是下一页的游标**（空窗 ⇒ `None`）。
    pub fn last(&self) -> Option<T> {
        self.n.checked_sub(1).and_then(|at| self.get(at))
    }

    /// **取窗那一侧用**：收一枚。收下了 ⇒ `true`；**已经满了** ⇒ `false` 并点亮
    /// [`Window::more`]（"这一趟没答完"）。
    ///
    /// **照实记（它替掉了三格半成品）**：账那一侧从前与 `Window` 同住一个模块，于是它直接
    /// 读写 `push` / `full` / 那个私有字段 `more`；账搬回实现侧之后跨了 crate，那三样不该
    /// 变成公开的可变面 ⇒ 收成这一手：**"塞不下了"就是"还有"**，一格判定、一处写。
    pub fn put(&mut self, id: T) -> bool {
        if self.full() {
            self.more = true;
            return false;
        }
        self.push(id);
        true
    }

    /// 收一枚（再满就丢：取窗那边收了 [`WINDOW_CAP`] 枚就停）。
    fn push(&mut self, id: T) {
        if let Some(slot) = self.items.get_mut(self.n) {
            *slot = Some(id);
            self.n += 1;
        }
    }

    /// 装满了。
    fn full(&self) -> bool {
        self.n == WINDOW_CAP
    }
}

// ── 码 ──────────────────────────────────────────────────────

/// 七条线上动作——**与核心那七条原语同名**：线上与模型是同一件事的两层，不该各起一套词。
pub const FOUND: u8 = 1;
pub const ENTER: u8 = 2;
pub const LEAVE: u8 = 3;
pub const AMID: u8 = 4;
pub const BAND: u8 = 5;
pub const BLOC: u8 = 6;
/// **代报名**：把**另一位**放进盟主自己立的那一枚盟（K2 翻案那一刀添的，见 [`Fail::NotChief`]）。
pub const ADMIT: u8 = 7;

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格：失败域那三格 + "读不懂"。
///
/// [`BAD`] 在失败表外（同板 / 树 / 身份服务那三家的先例）：它不是"哪个协议说的事"，
/// 是**这一问读不懂**。
///
/// **`4` 是新开的**（照实记）：本族原先只有两格失败（`UNKNOWN` / `FULL`），故 `3` 就收尾了。
/// 开面那一刀给失败域添了 [`Fail::Denied`]（"你手里那一枚门牌给不了这一条"），它按本族失败域
/// 的顺序排在这一格——**别家同一个概念排的是别的号**，那不是约定（同上面那张表的注）。
pub const UNKNOWN: u8 = 1;
pub const FULL: u8 = 2;
pub const BAD: u8 = 3;
pub const DENIED: u8 = 4;
/// 你不是这一枚盟的盟主（K2 翻案那一刀添的）：代报名只有**立它那位**做得成。
pub const NOT_CHIEF: u8 = 5;

// ── 帧骨架（两族同形的那一份）───────────────────────────────
//
// 长度、编 / 解、答话那几手**本体在 [`crate::frame`]**——coalition 与 principal 同形（这一族
// 的帧就是照它立的），故只有一份；这里只按本族的名字转出来（`mod.rs` 那一句
// 点名转出照旧，调用点一处都不用改）。

pub use crate::frame::{Query, Reply};

// ── 一问：一条动作一格 ──────────────────────────────────────

/// **一问的形状**——一条动作一格（同 `principal/frame.rs` 那条照实记：它替掉了
/// `pack_ask(op, a, b, back)` 那种"任何一枚码配上任何两格数"）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req {
    /// `FOUND`：铸一枚新盟——**两格都空**。
    Found,
    /// `ENTER`：进 `a` 那一枚盟。
    Enter(CoalitionId),
    /// `LEAVE`：离 `a` 那一枚盟。
    Leave(CoalitionId),
    /// `AMID`：`a` 此刻在 `b` 那一枚盟里吗（**两格都用**）。
    Amid(PrincipalId, CoalitionId),
    /// `BAND`：读 `a` 那一枚盟的盟籍；`b` = **游标**（从哪一枚之后接着读）。
    Band(CoalitionId, Option<PrincipalId>),
    /// `BLOC`：读 `a` 那一位在哪些盟里；`b` = **游标**。
    Bloc(PrincipalId, Option<CoalitionId>),
    /// `ADMIT`：把 `b` 那位放进 `a` 那一枚盟（**只有盟主叫得动**，见 [`Fail::NotChief`]）。
    ///
    /// **`b` 是 TID 不是身份号**（照实记）：这一格问的是"哪一位"，而运行期说得出口的"哪一位"
    /// 只有内核盖的那枚印章给得出（TID）——**解名**那一手在服务端（它本就有名册问面，见
    /// `programs/src/system/coalition/server.rs` 的 `who`）。线上因此不必先问一次名册再编帧。
    Admit(CoalitionId, TaskId),
}

impl Req {
    /// 编成线上那一形；`back` = **这一趟的回信孔在对端表里的号**（运输那一格，不是荷载）。
    pub fn query(self, back: PieToken) -> Query {
        let (op, a, b) = match self {
            Req::Found => (FOUND, 0, 0),
            Req::Enter(c) => (ENTER, c.get() as u64, 0),
            Req::Leave(c) => (LEAVE, c.get() as u64, 0),
            Req::Amid(p, c) => (AMID, p.get() as u64, c.get() as u64),
            Req::Band(c, after) => (BAND, c.get() as u64, cursor_of(after)),
            Req::Bloc(p, after) => (BLOC, p.get() as u64, cursor_of(after)),
            Req::Admit(c, target) => (ADMIT, c.get() as u64, target.get() as u64),
        };
        Query { op, a, b, back }
    }
}

/// **收进来的一问**（那两格号已经解成模型类型 / 游标）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Found,
    Enter(CoalitionId),
    Leave(CoalitionId),
    Amid(PrincipalId, CoalitionId),
    Band(CoalitionId, Option<PrincipalId>),
    Bloc(PrincipalId, Option<CoalitionId>),
    /// 代报名：盟号 ＋ **目标那一枚 TID**（解名在服务端）。
    Admit(CoalitionId, TaskId),
}

impl Wire {
    /// 解一问：`(读出来的动作, 回信孔那一格)`——**动作读不出来给内层那个 `None`**（表外的动作码：
    /// 这一问**有回信的路**，只是这一码我不认 ⇒ 持册者答一句 `BAD`）；**长度不对给外层那个
    /// `None`**（连"往哪回"都没有 ⇒ 不动账、也不回话）。
    pub fn take(bytes: &[u8]) -> Option<(Option<Wire>, PieToken)> {
        if bytes.len() != Query::LEN {
            return None;
        }
        let q = Query::fetch(bytes)?;
        let ask = match q.op {
            FOUND => Some(Wire::Found),
            ENTER => Some(Wire::Enter(CoalitionId::new(q.a as usize))),
            LEAVE => Some(Wire::Leave(CoalitionId::new(q.a as usize))),
            AMID => Some(Wire::Amid(
                PrincipalId::new(q.a as usize),
                CoalitionId::new(q.b as usize),
            )),
            BAND => Some(Wire::Band(
                CoalitionId::new(q.a as usize),
                cursor_in(q.b).map(|raw| PrincipalId::new(raw)),
            )),
            BLOC => Some(Wire::Bloc(
                PrincipalId::new(q.a as usize),
                cursor_in(q.b).map(|raw| CoalitionId::new(raw)),
            )),
            ADMIT => Some(Wire::Admit(
                CoalitionId::new(q.a as usize),
                TaskId::new(q.b as usize),
            )),
            // 表外的动作码：这一码不是我的（但"往哪回"读得出来）。
            _ => None,
        };
        Some((ask, q.back))
    }
}

// ── 窗：游标与一窗号 ────────────────────────────────────────

/// 游标那一格：**`b` = 游标 + 1**，`0` = 没有游标（从头取）。
///
/// 加一是那个双射：零号是真格子（`PrincipalId::ROOT` 是 0），拿 0 当"没有"会把它漏掉。
pub fn cursor_of<T: Id>(after: Option<T>) -> u64 {
    match after {
        Some(at) => at.get() as u64 + 1,
        None => 0,
    }
}

/// 游标那一格解回来（`0` ⇒ `None`；其余 ⇒ 裸号）。
pub fn cursor_in(b: u64) -> Option<usize> {
    if b == 0 { None } else { Some(b as usize - 1) }
}

// ── 一答：三种形状（格状态 ＋ 一格答 ＋ 一窗号）──────────────

/// 「格状态」那一形：失败那几格（[`UNKNOWN`] / [`FULL`] / [`BAD`]）只有这一格。
///
/// **照实记（为什么这一族多出这一形）**：成功那两形都带回荷载，失败没有——故线上有三种长度
/// （1 / 10 / `3 + 8n`），客侧按"我问的是哪一条"认。principal 那一面没有这一形：它的失败也占满
/// 10 字节（`Reply` 那一形）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「窗」那一形的**头三格**：状态 ＋ 未完 ＋ 条数（后面跟着那么多个号——那是尾巴，走
/// [`env::wire::store_tail`]）。
///
/// **"未完"那一格为什么只此一族有**：盟籍没有上限（一格盟可以有很多人）⇒ 窗装不下是常态；
/// 对照 operator 那一侧：一条 pane 本来就不超过 `PANE_CAP`，故那边不用带。
///
/// **`more` 那一格是真 `bool`**：只许 0 / 1 这条判据收在 [`env::wire::Field`] 一处
/// （`bool` 那一格），本族不再手写一遍、也没有"畸形的 2"这一形可读。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeqHead {
    pub status: u8,
    pub more: bool,
    pub count: u8,
}

/// 一答的上界：**最大那一形**（窗：头三格 ＋ [`WINDOW_CAP`] 枚号）。
pub const UNION_LEN: usize = SeqHead::LEN + WINDOW_CAP * 8;

/// 一窗号的**荷载**（编的那一侧用）：未完那一格 ＋ 一串**裸 8 字节号**。
///
/// **照实记（为什么存裸号、不存 `PrincipalId` / `CoalitionId`）**：两个号空间在这一格上
/// **分不开**（`band` 取的是身份号、`bloc` 取的是盟号，线上逐字同形），而 [`Union`] 得是**一枚
/// 具体类型**（服务端一处收尾：装一条、发一条）⇒ 不能按号泛型。与 operator 那格 `Word` 同一条
/// 口径：字段只管这一格多宽、怎么落字节，含义归问的人认。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seq {
    more: bool,
    len: usize,
    ids: [u64; WINDOW_CAP],
}

impl Seq {
    fn of<T: Id>(window: &Window<T>) -> Seq {
        let mut ids = [0u64; WINDOW_CAP];
        for (slot, id) in ids.iter_mut().zip(window.iter()) {
            *slot = id.get() as u64;
        }
        Seq {
            more: window.more(),
            len: window.len(),
            ids,
        }
    }

    fn ids(&self) -> &[u64] {
        self.ids.get(..self.len).unwrap_or(&[])
    }

    /// 按**问的那一族**把裸号造回来（读那一侧；编那一侧是 `of`）。
    pub fn window<T: Id>(&self) -> Window<T> {
        Window::gather(
            self.more,
            self.ids().iter().map(|raw| T::new(*raw as usize)),
        )
    }
}

/// **一答的形状**——三形：格状态（1）／一格答（10）／一窗号（`3 + 8n`）。
///
/// **照实记（名字）**：用户裁定这一族与树那一族同名同位——答的**形状**那一面叫 `Union`（同
/// `board::Req` 那句"一问的形状"，写成 `enum` 就是"几样里的一件"）。一格答那一形 [`Reply`]
/// 的本体在 [`crate::frame`]（两族同形）。
///
/// **照实记（这一族没有一个"原样的字节"读面——树那一族有）**：树那一族的四形**在线上分不开**
/// （"名"那一条长度即名长、另几形都以状态起头），故它把字节原样收下、由问的人认。这一族**不用
/// 那一手**：三形的长度互不相撞（`1` / `10` / `3 + 8n`，第三族全是 ≡ 3 mod 8，`n = 0..16`）⇒
/// **长度一说，形状就定了**。故 `In = Union`——与板那一族、与 [`Reply`] 同一条"写法与读法是
/// 同一个"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Union {
    /// 失败那几格（[`UNKNOWN`] / [`FULL`] / [`BAD`]）。
    Status(u8),
    /// 一格答：一枚号 / 是或非。
    One(Reply),
    /// 一窗号。
    Seq(Seq),
}

impl Union {
    /// 编一答：一窗号（**裸号进窗**——见 [`Seq`] 那条照实记）。
    pub fn seq<T: Id>(window: &Window<T>) -> Union {
        Union::Seq(Seq::of(window))
    }
}

impl Message for Union {
    /// **写法与读法是同一个**：形状由长度分得开（见 [`Union`] 那条照实记）。
    type In = Union;
    /// 这一族的缓冲：**最大那一形**（[`UNION_LEN`]）。
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match *self {
            Union::Status(code) => Status { status: code }.store_in(out),
            Union::One(reply) => reply.store_in(out),
            Union::Seq(seq) => {
                let head = SeqHead {
                    status: OK,
                    more: seq.more,
                    count: seq.len as u8,
                };
                let at = head.store_in(out)?;
                env::wire::store_tail(out, at, seq.ids())
            }
        }
    }

    /// 解一答：**先按长度分那一形**，再在该形自己的判据里解——不自洽 ⇒ `None`（不猜、不崩）。
    ///
    /// **照实记（从前那两条读法的次序搬进了这里，结果逐条相同）**：客侧那两条路各有一套次序
    /// ——`raw` 那条**先判长度**（恰好 [`Reply::LEN`] 才往下走，故一格状态那一形在那条路上读不出
    /// `FULL`），`read_seq` 那条**先看码**（非 `OK` ⇒ 那一格码，`FULL` 一路走到 `Fail::Full`）。
    /// 长度一分，两条次序都落在下面：**一格答那一形只认恰好 10**（`Status(FULL)` 落不进它），而
    /// **窗那一形先看码**（`Status(FULL)` 由它读成那一格码）。客侧那两条路各自认自己那一形
    /// （见 `coalition/client.rs`），判据一字未改。
    fn fetch(bytes: &[u8]) -> Option<Union> {
        match bytes.len() {
            Status::LEN => Some(Union::Status(Status::fetch(bytes)?.status)),
            Reply::LEN => Some(Union::One(Reply::fetch(bytes)?)),
            len if (SeqHead::LEN..=UNION_LEN).contains(&len) => {
                let head = SeqHead::fetch(bytes)?;
                let more = head.more;
                let count = head.count as usize;
                if count > WINDOW_CAP {
                    return None;
                }
                let body = bytes.get(SeqHead::LEN..)?;
                let mut ids = [0u64; WINDOW_CAP];
                let end = env::wire::fetch_tail(body, 0, &mut ids[..count])?;
                // **帧长即条数**：对不上就是不认（短一字节、条数说谎都落在这一句上）。
                if end != body.len() {
                    return None;
                }
                Some(Union::Seq(Seq {
                    more,
                    len: count,
                    ids,
                }))
            }
            _ => None,
        }
    }
}

// ── 失败域 ↔ 答话码 ─────────────────────────────────────────

crate::fail_codes! {
    /// 失败域 → 答话那一格（`None` = 一个失败都不是）。
    ///
    /// 四格：`Denied` 是开面那一刀添的、`NotChief` 是 K2 翻案那一刀添的（见那两格自己的注）。
    /// 数字按本族失败域的顺序排（`BAD` 收尾且在表外）——别家同一个概念排的是别的号，那不是约定。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::Full => FULL,
    Fail::Denied => DENIED,
    Fail::NotChief => NOT_CHIEF,
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────

/// 回信孔的记号：客人**每趟**铸一枚、借给 Server（这一趟的答话从它回来）。
///
/// 与另几面的 `*-back` 同一个形状、不同的记号：同一张表里两面的回信孔若刻同一个记号，
/// 就分不出这一枚是哪一面的。
pub const BACK: Mark = Mark::of("coalition-back");

/// **本族那块窗格在树上的路**：`/svc/sys/coalition`（头两段是四族共用的
/// [`crate::system::DIR`]，末段是本族自己的名字 [`NAME`]）——**一处说全**（同 principal）。
pub const DIR: Path = crate::system::DIR.join(NAME);

/// 本服务在树上的那一段名字（门的第二段）：`/svc/sys/coalition`——**它自己不是一格**（开面那一刀：
/// 两枚门牌是它底下那两格 `/svc/sys/coalition/{ask,set}`，末段名由
/// [`Grant::name`](super::grant::Grant::name) 给）。
pub const NAME: &str = "coalition";

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台那条 `the_three_back_marks_of_the_three_doors_do_not_collide`（那条判据随宿主靶
// 一并删了，用户裁定"protocol-case 没必要"）；**与名册那一对**钉在
// `crate::system::principal::frame`，**与线那一对**钉在 `lib.rs`——线那一枚住在
// `driver::line::frame`，而这一份**只认得 `env` 与同层 `core`**，看不见 `driver`。
const _: () = assert!(BACK.get() != Mark::NONE.get());
const _: () = assert!(BACK.get() != Mark::of(NAME).get());
