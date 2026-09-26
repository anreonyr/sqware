//! board 的**帧那一半** —— 帧、码、记号（内核那几只手的别名与两张会话失败域的映射
//! 在 `protocol` 那一侧的 `mod.rs`）。
//!
//! 本文件**不做裁决**：板上的规矩（谁能挂、挂哪儿、什么时候扫）全在 [`core`](super::core)。
//! 这里只有**编一帧 / 解一帧**与两张对照表（失败域 ↔ 答话码）。
//!
//! **照实记（这一份为什么拆出来）**：帧形今天只有机器在跑，而机器只走**顺路**——边角
//! （短帧 / 长帧 / 动作码不对 / 「这一码才带 seed」那一格 / 表外的码）一格都走不到。拆开是为了
//! 让那些边角在**宿主靶**上编得动；**那台靶已删**（用户裁定"protocol-case 没必要"）⇒ 这一份照旧
//! 只认 `env` 与同层 `core`，但那些边角今天**没有判据**。

use env::Mark;
use env::{PieToken, TaskId};

use super::core::Fail;

use crate::message::Message;

// 名字那一格用的是 `env::Name`（**定长名字那个类型**）——本模块另有一个 `Name`（只报名字
// 那一问的字段表），故这里一路写全路径：两个 `Name` 是同一件事的两层（值与帧），
// 谁都不该改名。

pub fn name_of(bytes: &[u8]) -> Option<env::Name> {
    let at = bytes.get(..env::wire::NAME_LEN)?;
    let mut raw = [0u8; env::wire::NAME_LEN];
    raw.copy_from_slice(&at[..]);
    env::Name::from_bytes(raw).ok()
}

// ── 四张字段表（**偏移一处都不写**）─────────────────────────
//
// ```text
//   Seed     [0] op   [1..33] name   [33..41] 入口号     （41 字节）
//   Name     [0] op   [1..33] name                       （33 字节）
//   Evict    [0] op                                      （1 字节）
//   Status   [0] status                                  （1 字节）
// ```
//
// 表名按**荷载**起：`Seed` 那一张多一格入口号、`Name` 那一张只报名字、`Evict` 空载荷、
// `Status` 是答话那一格。**不按动作、也不带 `Frame` 后缀**——它们住在本族的 `frame` 模块
// 里，这里每一个名字都是帧。
//
// **照实记（改名之前）**：三张表从前叫 `RegisterFrame` / `NameFrame` / `EvictFrame`——
// **动作名 ＋ 后缀**；而注销与查**共用**同一张表，两个动作名落在同一张表上，那个名字说不了
// 这件事（这张表要说的是"里头只有名字"）。
//
// 答话那一格的六个码见 [`OK`] / [`UNKNOWN`] / [`TAKEN`] / [`DENIED`] / [`FULL`] / [`BAD`]。
//
// 名字按 `NAME_LEN`(env::wire::NAME_LEN) 定长写（尾随 NUL 是填充）——**与牌子同
// 一个解码面**，故 `env::Name` 的读法全树只有一处。
//
// 那一格入口号是**客人把入口交出去之后、换回来的"种在板表里"的号**
// （`ship` 的返回值）——不是"客人的入口是几号"。两个编号空间不同源，互相拿错
// 正是旧树 `[33..41]` 那一格的病；**答案那一侧则干脆没有这一格**：查到的那枚入口经
// 会话交进客人的表，报文里再放一个号只会多出一份两边都得认的约定。

/// 登记那一问：动作码 ＋ 名字 ＋ 入口那 8 字节。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seed {
    pub op: u8,
    pub name: env::Name,
    pub seed: PieToken,
}

/// 只报名字那两问（注销 / 查）共用的形状。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Name {
    pub op: u8,
    pub name: env::Name,
}

/// 空载荷那一问（退场）：整帧只有动作码这一格。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Evict {
    pub op: u8,
}

/// **答话那一格**：整帧一格——答只有一句话（成功 / 四种失败 / 读不懂）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 报文的**上限**：登记那一枚最长（[`Seed`] 的宽度之和）。长度仍由每次 `push` 自己带
/// （孔不预设上限），这里只是**声明这一版只用多大**——一处上界。
pub const REQ_LEN: usize = Seed::LEN;

// 四个动作在报文里的码——**与核心那四个方法同名**（`register` / `unregister` /
// `lookup` / `evict`）：线上与模型是同一件事的两层，不该各起一套词。
//
// **它们不再是协议面**：编的那一侧由 [`Req`] 说、解的那一侧由 [`Wire`] 说，两枚码各被读一次
// （`Req::store` 写、`Req::fetch` 认）。外面认的是类型 ⇒ 降为私有——没有读者的格不留在面上。
const REGISTER: u8 = 1;
const UNREGISTER: u8 = 2;
const LOOKUP: u8 = 3;
// 第四格动作码：**空载荷**——退场那一句没有名字、也没有入口，故整帧只有这一字节。
const EVICT: u8 = 4;

/// 成功那一格：**全协议同一个号**——定义在 `contract/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 答话那一格。**前五格与 [`Fail`] 一一对应**，第六格不是
/// 失败域的：这一问读不懂（帧坏了 ⇒ 不猜、不崩）。
///
/// 数字是**线上的**，故与动作码同住一处；[`Fail`] 是模型那一侧的名字，两者的对照表只此
/// 一份（持有者那一侧编、客人那一侧读）。
pub const UNKNOWN: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENIED: u8 = 3;
pub const FULL: u8 = 4;
pub const BAD: u8 = 5;

crate::fail_codes! {
    /// 失败域 → 答话那一格。`None`（没失败）⇒ `OK`。
    ///
    /// 板这一侧原先这张表**住在程序侧**（`programs/.../board/server.rs` 里那个私有 `code`），
    /// 故协议层拿不到它——规则 1"一张负码表"就是被这一格破的。它现在与码表同住一处。
    bijective Fail; OK;
    Fail::Unknown => UNKNOWN,
    Fail::Taken => TAKEN,
    Fail::Denied => DENIED,
    Fail::Full => FULL,
}

/// **一问的形状**——一条动作一条形状：荷载收什么，帧里就写什么。
///
/// **照实记（它替掉了什么）**：从前是 `pack_ask(op: u8, name: Name, seed: Option<PieToken>)`
/// ——**任何一枚码都能配上任何一种荷载**：给退场那一码塞一个名字就编出一帧 41 字节的"退场"
/// （而线上那一句是**一字节短帧**），给查那一码配一个入口号也编得出来。四条动作与四张形状的
/// 关系原先活在**两张表**里（调用方那一处、板那一段 `match` 一处），错配**编得过**，症状要到
/// 读的那一侧才显形。
///
/// 现在：形状由类型说、编解码由字段表生成（[`Message`]），外面认的是类型——`pack_ask` /
/// `unpack_ask` 两枚自由函数随之退场。
///
/// **照实记（名字）**：这一族从前叫 `Ask` / `AskIn` / `Reply`。用户裁定 `Ask` / `Reply`
/// 这一对不要，用 **`Req` / `Wire` / `Union`**——故这里是新生的名字，不是改名；收的那一面
/// 是 [`Wire`]（它比这里多一格：表外的动作码）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req {
    /// 登记：名字 ＋ **板上那个入口号**（`ship` 换回来的那一枚——不是"客人的 Pie 是几号"）。
    Register { name: env::Name, seed: PieToken },
    /// 注销：只报名字（板按"开者 = 它"认领）。
    Unregister { name: env::Name },
    /// 查：只报名字。查到的那枚入口**经会话转授**，不从报文里走。
    Lookup { name: env::Name },
    /// 退场：**空载荷**，整帧一字节。
    Evict,
}

/// 收进来的一问——**动作码与荷载一起解**（帧里写着是哪一条动作，读的人不必先猜）。
///
/// **照实记（同词不同层：`Wire`）**：本仓有三处 `Wire`／`wire`——这里是**报文**那一层
/// （一族问话收进来是哪一个形状），另两处是 [`env::wire::Wire`]（六个寄存器的调用约定）
/// 与 [`env::wire::Field`]（**一种值**怎么落到字节上）。同名不同事，各自写在 doc 首行
/// （本仓对同词的既有做法，见 `protocol::session::slip` 的三个动词）。
///
/// 四种真动作各占一格；[`Wire::Unknown`] 单独一格，因为**板对它答的话与"读不懂"不同**
/// （见 `programs/src/system/board/server.rs` 的 `answer`）——[`Req`] 编不出它。
///
/// **照实记（名字）**：它从前叫 `AskIn`（`Ask` 的被动态），刀一里叫 `ReqIn`（`Req` 加尾巴）
/// ——那一个尾巴说不了它与 [`Req`] 的分别。用户裁定的第三个词是 `Wire`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Register {
        name: env::Name,
        seed: PieToken,
    },
    Unregister {
        name: env::Name,
    },
    Lookup {
        name: env::Name,
    },
    Evict,
    /// 表外的动作码：**这一帧读得懂（`[码][名字]`），但那一码不是这四枚之一**。
    Unknown,
}

impl Message for Req {
    type In = Wire;
    /// 这一族的缓冲就是最长那一枚（登记）。[`Seed::LEN`] 是**字段宽度之和**，不写数。
    type Buf = [u8; Seed::LEN];
    const EMPTY: Self::Buf = [0u8; Seed::LEN];

    /// 编进 `out`：**动作码由形状给**（不在别处再写一遍），偏移与长度由字段表求和。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match *self {
            Req::Register { name, seed } => Seed {
                op: REGISTER,
                name,
                seed,
            }
            .store_in(out),
            Req::Unregister { name } => Name {
                op: UNREGISTER,
                name,
            }
            .store_in(out),
            Req::Lookup { name } => Name { op: LOOKUP, name }.store_in(out),
            Req::Evict => Evict { op: EVICT }.store_in(out),
        }
    }

    /// 解开一问：**逐条动作比长度**（长度为该动作该有的长度是帧的契约，`store` 产出的就是
    /// 那个长度）——短一字节、长一字节都 ⇒ `None`（持板者据此答 `BAD`，不猜、不崩）。
    ///
    /// **空帧 ⇒ `None`**（连动作码都没有）。**表外的动作码 ⇒ [`Wire::Unknown`]**：那不是
    /// "读不懂"，是"这一码不是我的"——两件事板答的话不同，故分两格。
    ///
    /// 那一格入口号按 [`PieToken::NONE`]（0）= "不是号"解——令牌自 1 起，0 是内核的越界哨兵；
    /// **它是"这枚号合不合法"，不是"这一码带没带"**（带没带由 [`Req`] 说）。
    fn fetch(bytes: &[u8]) -> Option<Wire> {
        let op = *bytes.first()?;
        Some(match op {
            EVICT if bytes.len() == Evict::LEN => Wire::Evict,
            REGISTER if bytes.len() == Seed::LEN => {
                let frame = Seed::fetch(bytes)?;
                Wire::Register {
                    name: frame.name,
                    seed: frame.seed,
                }
            }
            UNREGISTER if bytes.len() == Name::LEN => Wire::Unregister {
                name: Name::fetch(bytes)?.name,
            },
            LOOKUP if bytes.len() == Name::LEN => Wire::Lookup {
                name: Name::fetch(bytes)?.name,
            },
            _ if !matches!(op, REGISTER | UNREGISTER | LOOKUP | EVICT) => Wire::Unknown,
            // 这四枚之一，长度却不是它该有的那个 ⇒ 读不懂。
            _ => return None,
        })
    }
}

// ── 答话那一侧 ──────────────────────────────────────────────

/// **一答的形状**——答只有一句话（[`Status`] 那一格里的码），故这一族没有第二条形状可用。
///
/// **照实记（为什么不拿 `Status` 直接当报）**：字段表是**字段**（那一格谁都能写），而外面要
/// 认的是"一答"这件事：`Union::of(code)` 收一句、`Union::get()` 取那一格。两层与 [`Req`] 那边
/// 同构——那里是**四张形状 ＋ 两个族类型**（`Req` / `Wire`），这里是**一张形状 ＋ 一个族
/// 类型**（`Union`）。
///
/// `type In = Union`：答的写法与读法是同一个。**表外的码不是"读不懂"**，它是一句答话的内容
/// （[`code_to_fail`] 对它答 `None`，读的人自己去认那一格）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Union {
    status: u8,
}

impl Union {
    /// 收一句答：那一格是线上的码（[`OK`] / [`UNKNOWN`] / [`TAKEN`] / …）。
    pub const fn of(status: u8) -> Self {
        Self { status }
    }

    /// 取那一格（认这枚码是读的人的事：见 [`code_to_fail`]）。
    pub const fn get(self) -> u8 {
        self.status
    }
}

impl Message for Union {
    type In = Union;
    /// 一答只有一格（[`Status::LEN`] = 1）。
    type Buf = [u8; Status::LEN];
    const EMPTY: Self::Buf = [0u8; Status::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Status {
            status: self.status,
        }
        .store_in(out)
    }

    /// 解一句答：**长度也是一格**（[`Status::LEN`]）——多一字节、少一字节都 ⇒ `None`
    /// （收答那一侧据此报 `Unknown`，与从前那一格 `Ok(1) => …` 同一判据）。
    fn fetch(bytes: &[u8]) -> Option<Union> {
        if bytes.len() != Status::LEN {
            return None;
        }
        Some(Union {
            status: Status::fetch(bytes)?.status,
        })
    }
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────
//
// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

/// 板那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "board";

/// 注册入口那一枚孔上的记号（**两侧同一个**：客人铸它时刻上去的，板按它把入口与问话孔
/// 分开——两枚都是客人铸的、都是客人交来的，只有记号分得开）。
pub const ENTRY_MARK: Mark = Mark::of("entry");

/// 问话孔那一枚上的记号（同上：客人铸、客人交；板按它认领那枚孔）。
///
/// **带面名**（`board-ask`）：认领键是"谁开的 + 记号"，而同一枚任务可能同时是两面的客人
/// ——两枚孔都铸在它自己那张表里，记号再一样就分不开。理由与实测见
/// `protocol::system::operator::frame::ASK_MARK`。
pub const ASK_MARK: Mark = Mark::of("board-ask");

/// 提示孔那一枚上的记号（板线程铸它时刻上去的；装配者按它认领那一枚）。
pub const TIP_MARK: Mark = Mark::of("tip");

/// 提示之路的名字（只有装配者那侧用得上：板线程那一枚是它自己铸的，不需要名字）。
pub const TIP_NAME: &str = "board-tip";

/// 提示那一格的载荷：**客人号（8 字节）＋ 定长名字 `NAME_LEN` ＋ 答话路那一格
/// `PieToken::WIDTH`**——装配者往那条路上推的就是这一条记录（一句话：**来客人了，它是谁**，
/// 以及**它的答话路在我表里是几号**）。
///
/// **照实记（末格为什么在，以及它替掉了什么）**：它从前不在——板拿到提示之后要**扫自己的表**
/// 找回那一枚（`server.rs::reply_of`：来源 = 装配者 ＋ 开者 = 这位客人 ＋ 记号 = 板路）。那一扫
/// 是每位客人**每趟事件**一遍全表，而枚举每一枚还要算一次 `vestor`（吃全世界快照 + 一次分配）：
/// 读数见提交 `b58fda4`。装配者**本来就有**这个号（`port::ship` 的 `to.seed()`，原先在
/// `bridge.rs::hand` 里被 `.map(|_| ())` 扔掉），故让它随提示一起过来、板一次 `Reserve` 验完——
/// 判据一字没改（开者 ＋ 记号）。
///
/// **照实记（名字为什么从这一格走，而不是等客人 `REGISTER`）**：死亡道是**装配者**铸的
/// （记号 `gone-<名字>` 照装配单写），故"这一位叫什么"它本来就有；而板要认领那一条道，
/// 只能按名字（[`LANE_PREFIX`]）。从前板只能等客人自己在 `REGISTER` 里报名字 ⇒
/// **没登记的客人死了也没人报**。实测（临时探针：让结盟服务在起手之后死掉）：
/// `board: swept n=1` 有、**`system: gone coalition` 一条都没有**——三枚内件与三台驱动都
/// 不登记。名字搭提示这一格过来之后，板在 `admit` 那一刻就把"谁 → 道"记下，
/// **与客人登不登记无关**；`REGISTER` 从此只管"名字 → 入口"那一件事。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tip {
    pub who: TaskId,
    pub name: env::Name,
    pub reply: PieToken,
}

/// 死亡道的记号前缀：**一位客人一条**（`gone-<名字>`），由装配者铸、各交一份给板。
///
/// 一客人一道 ⇒ **身份就是"哪条道响了"**：两位同时死也不会挤在一格上丢名字，装配者那边
/// 也不必按名字猜。板按这位客人的**名字**找回那一条——名字从提示那一格来（[`Tip::LEN`]），
/// 不是从牌子上读的（牌子会被惰性摘掉，而 `admit` 那一刻名字刚到）。
pub const LANE_PREFIX: &str = "gone-";

// ── 面不相撞（**编译期**钉住——用户裁定"常量交给编译器"）────────────────────
//
// 原先这是宿主台的一条运行时用例（`the_board_marks_and_the_lane_prefix_are_what_they_say`）。
// 那一格里真会撞的只有下面这几对；余下几条（`LINK == "board"` / `TIP_NAME == "board-tip"` /
// `LANE_PREFIX == "gone-"`）比的是**常量自己的定义式**，是同义反复，故随用例一起去掉。
const _: () = assert!(ASK_MARK.get() != Mark::of("operator-ask").get());
const _: () = assert!(ASK_MARK.get() != Mark::of("ask").get());
const _: () = assert!(ASK_MARK.get() != TIP_MARK.get());
const _: () = assert!(TIP_MARK.get() != Mark::of(TIP_NAME).get());
const _: () = assert!(ENTRY_MARK.get() != Mark::NONE.get());
