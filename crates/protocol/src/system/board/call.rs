//! board 的**适配那一半** —— 内核那几只手的别名、立板、交出，与两张会话失败域的映射。
//!
//! 帧与码见 [`frame`](super::frame)；`pub use super::frame::*;` 把那一整片照旧转出来 ⇒
//! **调用点一处都不用改**（`board::call::pack_ask`、`board::call::LINK`、
//! `board/mod.rs` 里那句 `pub use call::{…}` 全都照旧）。
//!
//! 判据只有一条可机械检查的纪律——
//!
//! > 本文件里的 `if` / `match` **一处裁决也没有**，只有三件事：一个 `0` 哨兵
//! > （[`opened_by`]）、"这一码才带 seed"（[`pack_ask`]）与两张对照表
//! > （[`map_claim`] / [`map_seat`]）。
//!
//! （旧注写的是"这里不出现 `if` / `match`"——**照实记：`map_*` 那两张表与它同一次落地，
//! 那句话从写下的第一天起就是假的**。）

use env::{PieToken, TaskId};

use contract::system::board::desk::Desk;
use super::core::{Board, Fail, Unship, VestedBy};
use crate::session::{Claim, Seat};

pub use super::frame::*;

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

/// **立一本账**（一位客人一格）：把"读内核事实"的那一枚接上（`vested_by`，`Reserve` 那一问）
/// ——**账住「约」，手在「口」**，这一手就是那个接口。
pub const fn desk() -> Desk {
    Desk::new(vested_by)
}

/// **交出**：把调用方手里那枚入口交给持板者（`Accord` 一份副本），返"种在持板者表里"的号；
/// 反过来的那一半（把板上那一份转授给调用方，`Query` 的下场）**是同一件事**，故同一个名字
/// ——照实记：这两个方向原先叫 `hang` 与 `give`，收口那一刀并成了这一个。
///
/// 这就是"谁挂的"的来历：板上那枚是**亲手交出去的**，故 [`vested_by`] 认得出谁授的它。
/// 权限给满（`R|W`）**加一格 `VEST`**：入口要能用来说话，而持板者的本职就是**再授出**
/// （`Query` 的下场）——内核那道"持 `VEST` 才交得出去"的闸（`Need::Grant`）挡的就是
/// "板查到了却授不出去"；拿到它的人把它转给第三方是常态（那正是"一个名字指向一个入口"
/// 的用法），故这里也不替调用方裁剪。
///
/// 身体在 [`crate::session::call::ship`]（**同名的裸手**）；**失败域是本模块的**
/// （`Denied`）：身体共用，失败值各自说（与 [`map_claim`] / [`map_seat`] 同款）。
pub fn ship(entry: PieToken, to: TaskId) -> Result<PieToken, Fail> {
    crate::session::call::ship(entry, to).map_err(|()| Fail::Denied)
}

/// 牌子上的名字（**定长解码面**：尾随 NUL 是填充，不是内容）。

pub fn map_claim(claim: Claim) -> Fail {
    match claim {
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
        Seat::Full => Fail::Full,
    }
}
