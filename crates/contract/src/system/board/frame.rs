//! board 的**帧那一半** —— 帧、码、记号（内核那几只手的别名与两张会话失败域的映射
//! 在 `protocol` 那一侧的 `call`）。
//!
//! 本文件**不做裁决**：板上的规矩（谁能挂、挂哪儿、什么时候扫）全在 [`core`](super::core)。
//! 这里只有**编一帧 / 解一帧**与两张对照表（失败域 ↔ 答话码）。
//!
//! **照实记（这一份为什么拆出来）**：帧形今天只有机器在跑，而机器只走**顺路**——边角
//! （短帧 / 长帧 / 动作码不对 / 「这一码才带 seed」那一格 / 表外的码）一格都走不到。拆开之后
//! 这一份**只认 `env` 与同层 `core`**，宿主靶能把它逐字编进去跑判据。

use env::Mark;
use env::{Name, PieToken};

use super::core::Fail;

pub fn name_of(bytes: &[u8]) -> Option<Name> {
    let at = bytes.get(..env::wire::NAME_LEN)?;
    let mut raw = [0u8; env::wire::NAME_LEN];
    raw.copy_from_slice(&at[..]);
    Name::from_bytes(raw).ok()
}

// ── 一问一答那一步（`Query` 的载体）──────────────────────────

/// 一问一答的帧：**一问一答各一句**。
///
/// ```text
///   Query   [0] op   [1..33] name   [33..41] 入口号（只有 Register 用）
///   Short   [0] op                                （Dismiss：整帧一字节，空载荷）
///   Reply   [0] status
/// ```
///
/// 答话那一格的六个码见 [`OK`] / [`UNKNOWN`] / [`TAKEN`] / [`DENIED`] / [`FULL`] / [`BAD`]。
///
/// 名字按 `NAME_LEN`(env::wire::NAME_LEN) 定长写（尾随 NUL 是填充）——**与牌子同
/// 一个解码面**，故 `Name` 的读法全树只有一处。
///
/// 那一格入口号是**客人把入口交出去之后、换回来的"种在板表里"的号**
/// （`ship` 的返回值）——不是"客人的入口是几号"。两个编号空间不同源，互相拿错
/// 正是旧树 `[33..41]` 那一格的病；**答案那一侧则干脆没有这一格**：查到的那枚入口经
/// 会话交进客人的表，报文里再放一个号只会多出一份两边都得认的约定。
///
/// 报文的**上限就是 [`ASK_LEN`]**：本协议只有两种帧——这一种有载荷的（[`REGISTER`] /
/// [`UNREGISTER`] / [`LOOKUP`]）与 [`EVICT`] 的一字节短帧。长度仍由每次 `push` 自己带
/// （孔不预设上限），这里只是**声明这一版只用多大**——一处上界。
pub const ASK_LEN: usize = 1 + env::wire::NAME_LEN + 8;

/// 四个动作在报文里的码——**与核心那四个方法同名**（`register` / `unregister` /
/// `lookup` / `evict`）：线上与模型是同一件事的两层，不该各起一套词。
pub const REGISTER: u8 = 1;
pub const UNREGISTER: u8 = 2;
pub const LOOKUP: u8 = 3;
/// 第四格动作码：**空载荷**——退场那一句没有名字、也没有入口，故整帧只有这一字节
/// （给它塞两格空位就白要 40 字节，见 [`op_of`] 与 [`unpack_ask`] 的分工）。
pub const EVICT: u8 = 4;

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

/// 把一问编成字节。`seed` 只有 [`REGISTER`] 用得上。
pub fn pack_ask(op: u8, name: Name, seed: Option<PieToken>) -> [u8; ASK_LEN] {
    let mut out = [0u8; ASK_LEN];
    out[0] = op;
    out[1..1 + env::wire::NAME_LEN].copy_from_slice(name.bytes());
    if let Some(seed) = seed {
        out[1 + env::wire::NAME_LEN..].copy_from_slice(&seed.to_bytes());
    }
    out
}

/// 只读第一格**动作码**——短帧也读得动，故分流先问这一句。空帧 ⇒ `None`
/// （持板者据此答 `BAD`，不猜、不崩）。
pub fn op_of(bytes: &[u8]) -> Option<u8> {
    bytes.first().copied()
}

/// 解开一问的**载荷**：`(名字, 那一格入口号)`。**读不懂返 `None`**（持板者据此答 `BAD`，
/// 不猜、不崩）。
///
/// 只在**有载荷**的那几码上叫（[`op_of`] 已经分过流：退场那一句是一字节短帧，不进这里）。
/// 长度为 [`ASK_LEN`] 是**帧的契约**（`pack_ask` 产出的就是这个长度），故短一字节即读不懂。
/// 那一格入口号按 [`PieToken::NONE`] = "没带"解——令牌自 1 起，0 是内核的越界哨兵。
pub fn unpack_ask(bytes: &[u8]) -> Option<(Name, PieToken)> {
    let name = name_of(bytes.get(1..)?)?;
    let at = bytes.get(1 + env::wire::NAME_LEN..ASK_LEN)?;
    Some((name, PieToken::from_bytes(at)?))
}

/// 会话的失败域 → 板的失败域：**"它不在"是一条判据**，故两边只留一个名字
/// （`Fail::Unknown`）。
///
/// 「一笔都没到」与「到了一些、不齐」在上面那一层都归 `Unknown`/`Full`：板这一侧
/// 只有一格答话码，问的人按它决定要不要重问。

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
/// `protocol::system::operator::call::ASK_MARK`。
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
pub const TIP_LEN: usize = 8 + env::wire::NAME_LEN + env::PieToken::WIDTH;

/// 死亡道的记号前缀：**一位客人一条**（`gone-<名字>`），由装配者铸、各交一份给板。
///
/// 一客人一道 ⇒ **身份就是"哪条道响了"**：两位同时死也不会挤在一格上丢名字，装配者那边
/// 也不必按名字猜。板按这位客人的**名字**找回那一条——名字从提示那一格来（[`TIP_LEN`]），
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
