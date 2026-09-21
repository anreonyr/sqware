//! board 的转发层 —— 内核那几只手的别名。
//!
//! 本文件**不做裁决**：板上的规矩（谁能挂、挂哪儿、什么时候扫）全在
//! [`core`](super::core)。这里只做三件事：**转发一次**、把"不在我表里"翻成 [`None`]、
//! 把错误码翻成"没成"。
//!
//! 判据只有一条可机械检查的纪律——
//!
//! > 本文件里的 `if` / `match` **一处裁决也没有**，只有三件事：一个 `0` 哨兵
//! > （[`opened_by`]）、"这一码才带 seed"（[`pack_ask`]）与两张对照表
//! > （[`map_claim`] / [`map_seat`]）。
//!
//! （旧注写的是"这里不出现 `if` / `match`"——**照实记：`map_*` 那两张表与它同一次落地，
//! 那句话从写下的第一天起就是假的**。）
//!
//! 板服务（板侧待客 / 客侧问一句）住在同一个模块的三侧文件里（`server` / `client` / `bridge`）：
//! 本模块只给**两边都要用的那几手**（认来源 / 认出这扇门是谁的 / 授出 / 收下 / 帧）。

use env::{Name, PieToken, TaskId};

use super::core::{Board, Fail, Unship, VestedBy};

use crate::session::{Claim, Seat};

// ── 一个调用的三个事实：身体在 `session::call`，这里只取名字 ──────────
//
// 三格是**一组**，三个名字读成同一句式的被动式事实、故等长（9/9/9）：
// **这枚是谁授的 / 这扇门是谁开的 / 这枚被标成什么**。板这一侧原先各抄一份
// （`probe`5 / `opened_by`9 / `mark_of`7——不等长本身就是"这一组还没想清楚"的信号），
// 那一份已删；本模块要讲的话堆在下面这一段。
pub use crate::session::call::{marked_as, opened_by, vested_by};

/// 自释一份：**装运 / 卸下**——`ship` 的反面。牌子被换掉或扫空时用它，
/// 否则那枚门闩漏在板上。身体在 [`crate::session::call::unship`]。
pub use crate::session::call::unship;

/// 立一块板：把两枚机制函数交给核心（核心因此不 `use` 内核）。
///
/// `const` 是为了它能当 `static` 的初值：板只有一份，住在板那一台（`super::server`）。
pub const fn board() -> Board {
    let vested_by: VestedBy = vested_by;
    let unship: Unship = unship;
    Board::new(vested_by, unship)
}

/// 挂上：把调用方手里那枚入口**交给持板者**（`Accord` 一份副本），返"种在持板者表里"的号。
///
/// 这就是"谁挂的"的来历：板上那枚是**亲手交出去的**，故 [`vested_by`] 认得出谁授的它。
/// 权限给满（`R|W`）**加一格 `VEST`**：入口要能用来说话，而持板者的本职就是**再授出**
/// （`Query` 的下场）——内核那道"持 `VEST` 才交得出去"的闸（`Need::Grant`）挡的就是
/// "板查到了却授不出去"。身体在 [`crate::session::call::ship`]。
pub use crate::session::call::ship as hang;

/// 授出：把板上那一份入口转授给调用方（`Query` 的下场）。
///
/// 与 [`hang`] 同一份子集（`R|W|VEST`）：**入口可以再传**——拿到它的人把它转给第三方
/// 是常态（那正是"一个名字指向一个入口"的用法），故这里不替调用方裁剪。
/// **失败域是本模块的**（`Denied`）：身体共用，失败值各自说。
pub fn give(entry: PieToken, to: TaskId) -> Result<PieToken, Fail> {
    crate::session::call::ship(entry, to).map_err(|()| Fail::Denied)
}

/// 牌子上的名字（**定长解码面**：尾随 NUL 是填充，不是内容）。
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
/// 名字按 [`NAME_LEN`](env::wire::NAME_LEN) 定长写（尾随 NUL 是填充）——**与牌子同
/// 一个解码面**，故 `Name` 的读法全树只有一处。
///
/// 那一格入口号是**客人把入口交出去之后、换回来的"种在板表里"的号**
/// （[`hang`] 的返回值）——不是"客人的入口是几号"。两个编号空间不同源，互相拿错
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

/// 答话那一格。**前五格与 [`Fail`] 一一对应**（`OK` = 一个失败都不是），第六格不是
/// 失败域的：这一问读不懂（帧坏了 ⇒ 不猜、不崩）。
///
/// 数字是**线上的**，故与动作码同住一处；[`Fail`] 是模型那一侧的名字，两者的对照表只此
/// 一份（持有者那一侧编、客人那一侧读）。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const TAKEN: u8 = 2;
pub const DENIED: u8 = 3;
pub const FULL: u8 = 4;
pub const BAD: u8 = 5;

/// 失败域 → 答话那一格。`None`（没失败）⇒ `OK`。
///
/// 板这一侧原先这张表**住在程序侧**（`programs/.../board/server.rs` 里那个私有 `code`），
/// 故协议层拿不到它——规则 1"一张负码表"就是被这一格破的。它现在与码表同住一处。
pub const fn fail_to_code(fail: Option<Fail>) -> u8 {
    match fail {
        None => OK,
        Some(Fail::Unknown) => UNKNOWN,
        Some(Fail::Taken) => TAKEN,
        Some(Fail::Denied) => DENIED,
        Some(Fail::Full) => FULL,
    }
}

/// 线上答话那一格 → 失败域。`OK`（没失败）与 `BAD`（这一问读不懂）**都不是失败域里的
/// 东西**，故两者同一格答 `None`——读的人靠 [`op_of`] / [`unpack_ask`] 先分流。
///
/// **本表是双射**（四个失败一格一码），故反向答得回来。
pub const fn code_to_fail(code: u8) -> Option<Fail> {
    match code {
        UNKNOWN => Some(Fail::Unknown),
        TAKEN => Some(Fail::Taken),
        DENIED => Some(Fail::Denied),
        FULL => Some(Fail::Full),
        _ => None,
    }
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
pub fn map_claim(claim: Claim) -> Fail {
    match claim {
        // 我的表读不动 ⇒ 这一问没有答案（与"它不在"同一格：都不是"板答了没有"）。
        Claim::Unread => Fail::Unknown,
        Claim::Timeout => Fail::Unknown,
        Claim::Partial => Fail::Full,
    }
}

/// 装一条路的失败域 → 板的失败域。
///
/// 与 [`map_claim`] 同一条口径：名字/资源上的毛病（名字非法、同名已装、铸不出孔）是
/// **调用方写错了** ⇒ `Denied`；交不出去（对端已不在）⇒ `Unknown`（"它不在"）；
/// 账腾不出来 ⇒ `Full`。
pub fn map_seat(seat: Seat) -> Fail {
    match seat {
        Seat::NoName => Fail::Denied,
        Seat::NoHole => Fail::Denied,
        Seat::NoSeed => Fail::Unknown,
        Seat::NoRoom => Fail::Full,
    }
}

// ── 载体两侧共用的坐标 ─────────────────────────────────────
//
// 这几格是**记号与名字**：两侧都要按它认领/铸孔，故只能有一份（规则 5）。

/// 板那条通道的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const LINK: &str = "board";

/// 注册入口那一枚孔上的记号（**两侧同一个**：客人铸它时刻上去的，板按它把入口与问话孔
/// 分开——两枚都是客人铸的、都是客人交来的，只有记号分得开）。
pub const ENTRY_MARK: &str = "entry";

/// 问话孔那一枚上的记号（同上：客人铸、客人交；板按它认领那枚孔）。
pub const ASK_MARK: &str = "ask";

/// 提示孔那一枚上的记号（板线程铸它时刻上去的；装配者按它认领那一枚）。
pub const TIP_MARK: &str = "tip";

/// 提示之路的名字（只有装配者那侧用得上：板线程那一枚是它自己铸的，不需要名字）。
pub const TIP_NAME: &str = "board-tip";

/// 死亡道的记号前缀：**一位客人一条**（`gone-<名字>`），由装配者铸、各交一份给板。
///
/// 一客人一道 ⇒ **身份就是"哪条道响了"**：两位同时死也不会挤在一格上丢名字，装配者那边
/// 也不必按名字猜。板按这位客人的**名字**（从它留在板上的牌子上读）找回那一条。
pub const LANE_PREFIX: &str = "gone-";
