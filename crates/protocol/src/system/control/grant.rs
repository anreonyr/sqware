//! 面那一维：一枚 Grant = 一条权柄边界。

use super::frame::Wire;

crate::table! {
/// **一条权柄边界**：一枚 = 一面。四位，位次 1..=4
    pub enum Grant {
/// **问**：这一条此刻处于哪个阶段（`State`）——只读，不动机器一根手指
        State => "state", (Wire::State(_));
/// **造**：按名字起一条（建域 ＋ 产代表线程，恒产未放行）
        Mint => "mint", (Wire::Mint(_));
/// **放行**：放行 ＋ 等就绪
        Embark => "embark", (Wire::Embark(_));
        Debark => "debark", (Wire::Debark(_));
        Ruin => "ruin", (Wire::Ruin(_));
    }
    stem: "control-entry-",
    wire_ty: Wire,
}
