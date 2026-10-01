//! operator::grant —— **操作面那一维**：一枚 `Grant` = 一枚操作。
//! ```text
//!   part 分   land 落   find 寻   trim 剪   list 列   seek 译   name 名      ← 七条原语
//!   Face 一面持树者（全操作面）   Grant 一柄授面的权（**一枚 = 一枚操作**）
//! ```
//! 三位一体一句话：**来源 = 树上那一格，判别 = 会话说的是哪一位**。故

use super::frame::Wire;

crate::faces! {
    /// **一柄授面的权**：一枚 = 一枚操作。七位，位次 1..=7。
    pub enum Grant {
        /// `part` 分。
        Part => "part",
        /// `land` 落。
        Land => "land",
        /// `find` 寻。
        Find => "find",
        /// `trim` 剪。
        Trim => "trim",
        /// `list` 列。
        List => "list",
        /// `seek` 译。
        Seek => "seek",
        /// `name` 名。
        Name => "name",
    }
    stem: "operator-ask-",
    name_max: 4,
    wire_ty: Wire,
    wire: {
        Wire::Part { .. } => Part,
        Wire::Land { .. } => Land,
        Wire::Find(_) => Find,
        Wire::Trim(_) => Trim,
        Wire::List(_) => List,
        Wire::Road(_) => Seek,
        Wire::Name(_) => Name,
    }
}
