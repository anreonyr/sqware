//! operator 的**适配那一半** —— 内核那几只手的别名、立树、交出，与两张会话失败域的映射。
//!
//! 帧与码见 [`frame`](super::frame)；`pub use super::frame::*;` 把那一整片照旧转出来 ⇒
//! **调用点一处都不用改**（`operator::call::Ask`、`operator::call::LINK`、
//! `operator/mod.rs` 里那句 `pub use call::{…}` 全都照旧）。

use env::{PieToken, TaskId};

use contract::system::operator::desk::Desk;
use super::core::{Fail, Operator, Stamps, Unship};
use crate::session::{Claim, Seat};

pub use super::frame::*;

/// 会话的失败域 → 树的失败域：**"它不在"是一条判据**，故两边只留一个名字
/// （[`Fail::Unknown`]）。
///
/// 「一笔都没到」与「到了一些、不齐」在上面那一层都归 `Unknown` / `Full`：树这一侧只有
/// 一格答话码，问的人按它决定要不要重问。
pub fn map_claim(claim: Claim) -> Fail {
    match claim {
        Claim::Timeout => Fail::Unknown,
        Claim::Partial => Fail::Full,
    }
}

/// 装一条路的失败域 → 树的失败域。
///
/// 名字 / 资源上的毛病（名字非法、同名已装、铸不出孔）是**调用方写错了** ⇒ `Unknown`
/// （树上没有这一格可指）；交不出去（对端已不在）⇒ `Unknown`（"它不在"）；账腾不出来 ⇒ `Full`。
pub fn map_seat(seat: Seat) -> Fail {
    match seat {
        Seat::NoName => Fail::Unknown,
        Seat::NoHole => Fail::Unknown,
        Seat::NoSeed => Fail::Unknown,
        Seat::Full => Fail::Full,
    }
}

// ── 一个调用的三个事实：身体在 `session::call`，这里只取名字 ──────────
//
// 三格是**一组**，三个名字读成同一句式的被动式事实、故等长（9/9/9）：
// **这枚是谁授的 / 这扇门是谁开的 / 这枚被标成什么**。树这一侧原先各抄一份
// （`probe`5 / `opened_by`9 / `mark_of`7），那一份已删（三格上三处的读法见 `vested_by`）。
pub use crate::session::call::{marked_as, opened_by, vested_by};

/// **卸下**：自释一份。剪掉或换掉一枚 `Tile` 时由核心叫它。
pub use crate::session::call::unship;

/// 立一棵树：把注入的机制交给核心（核心因此不 `use` 内核）。
///
/// `const` 是为了它能当 `static` 的初值——树只有一棵，住在本域（`bin/operator`）。
pub const fn tree() -> Operator {
    let stamps: Stamps = Stamps {
        vested_by,
        opened_by,
    };
    let unship: Unship = unship;
    Operator::new(stamps, unship)
}

/// **立一本账**（一位客人一格）：把"读内核事实"的那一枚接上——账住「约」，手在「口」。
pub fn desk() -> Desk {
    Desk::new(vested_by)
}

/// **交出**：把调用方手里那一枚交给持树者（`Accord` 一份副本），返"种在持树者表里"的号；
/// 反过来的那一半（持树者把树上那一枚转授给客人，`find` 的下场）**是同一件事**，故同一个名字
/// ——照实记：这两个方向原先叫 `hang` 与 `give`，收口那一刀并成了这一个。
///
/// 权限给满（`R|W`）**加一格 `VEST`**：持树者查到名字时要**再授出**（`find` 的下场）——
/// 内核那道"持 `VEST` 才交得出去"的闸挡的就是"查到了却授不出去"；拿到它的人可以再传
/// ——那正是"一个名字指向一枚 Pie"的用法，故这里也不替调用方裁剪。
///
/// 身体在 [`crate::session::call::ship`]（**同名的裸手**）；**失败域是本模块的**
/// （`Unknown`）：身体共用，失败值各自说（与 [`map_claim`] / [`map_seat`] 同款）。
pub fn ship(entry: PieToken, to: TaskId) -> Result<PieToken, Fail> {
    crate::session::call::ship(entry, to).map_err(|()| Fail::Unknown)
}
