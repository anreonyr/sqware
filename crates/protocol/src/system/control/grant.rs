//! control::grant —— **面那一维**：一枚 `Grant` = 一条权柄边界。
//! ```text

use super::frame::Wire;

crate::faces! {
    /// **一条权柄边界**：一枚 = 一面。四位，位次 1..=4。
    pub enum Grant {
        /// **问**：这一条此刻处于哪个阶段（`State`）——只读，不动机器一根手指。
        State => "state",
        /// **造**：按名字起一条（建域 ＋ 产代表线程，恒产未放行）。
        Mint => "mint",
        /// **放行**：放行 ＋ 等就绪。
        Start => "start",
        Stop => "stop",
    }
    stem: "control-entry-",
    name_max: 5,
    wire_ty: Wire,
    wire: {
        // 四条线上码逐条说它落哪一面——**一码一面，不并 `|`，也不并面**（用户裁定）。
        Wire::State(_) => State,
        Wire::Mint(_) => Mint,
        Wire::Start(_) => Start,
        Wire::Stop(_) => Stop,
    }
}
