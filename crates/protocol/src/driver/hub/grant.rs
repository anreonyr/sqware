//! hub::grant — **面那一维**：一枚 `Grant` = 一条权柄边界。
//!
//! ```text
//!   Bond 报名    List 列册    Claim 认领        ← 三条原语，一原语一面
//!   位次 1..=3：Bond / List / Claim             → /svc/hub/{bond,list,claim}
//! ```
//!
//! **`Claim` 那一面有正本与每台那一份**：正本挂在 `/svc/hub/claim`（发现入口与权柄边界），而
//! 它的**每一台那一份**挂在 `/dev/<类>/<名>` 那一格上（`permit = Among(c_类)`）。三枚面共一个
//! 词根、各一段名字——面名只有一处（[`Grant::name`]）。
//!
//! **回信孔那一枚不是面**：它每趟自带、与哪一面无关，故不在这张表里（它的记号见
//! [`super::frame::BACK_MARK`]，与三枚面**都不相撞**——下面那三句编译期断言钉住）。

use super::frame::{ALIVE_MARK, BACK_MARK, Wire};

crate::faces! {
    /// **一条权柄边界**：一枚 = 一面。三位，位次 1..=3。
    pub enum Grant {
        /// **报名**：许我驱这一类（幂等）。
        Bond => "bond",
        /// **列册**：这一类里现在有哪几台、哪几台有主（纯读）。
        List => "list",
        /// **认领**：这台归我（在**每台那一份**上问）。
        Claim => "claim",
    }
    stem: "hub-entry-",
    name_max: 5,
    wire_ty: Wire,
    wire: {
        // 三条线上码逐条说它落哪一面——**一码一面，不并 `|`**。
        Wire::Bond(_) => Bond,
        Wire::List(_, _) => List,
        Wire::Claim { .. } => Claim,
    }
}

// ── 回信孔 / 报活孔那两枚与三枚面**都不相撞**（编译期钉住）────────────────
//
// 比的是 `.get()` 那个裸值：`Mark` 的 `PartialEq` 不是 `const`，而 `get` 是 `const fn`
// （同 line 那一族的两句）。
const _: () = assert!(BACK_MARK.get() != Grant::Bond.mark().get());
const _: () = assert!(BACK_MARK.get() != Grant::List.mark().get());
const _: () = assert!(BACK_MARK.get() != Grant::Claim.mark().get());
// 报活孔也在这一族那两条路上走（一条是"面"、一条是本域铸的孔）——同一张表里分不开
// 就是同一枚，故一并钉住（同 `line::frame` 那一族的三句）。
const _: () = assert!(ALIVE_MARK.get() != Grant::Bond.mark().get());
const _: () = assert!(ALIVE_MARK.get() != Grant::List.mark().get());
const _: () = assert!(ALIVE_MARK.get() != Grant::Claim.mark().get());
const _: () = assert!(ALIVE_MARK.get() != BACK_MARK.get());
const _: () = assert!(ALIVE_MARK.get() != env::Mark::NONE.get());
