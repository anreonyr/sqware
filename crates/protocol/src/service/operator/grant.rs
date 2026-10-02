//! 操作面那一维：一枚 Grant = 一枚操作。
//! 三位一体一句话：**来源 = 树上那一格，判别 = 会话说的是哪一位**。

use super::frame::Wire;

crate::table! {
/// **一柄授面的权**：一枚 = 一枚操作。八位，位次 1..=8
    pub enum Grant {
/// `part` 分
        Part => "part", (Wire::Part { .. });
/// `land` 落
        Land => "land", (Wire::Land { .. });
/// `find` 寻
        Find => "find", (Wire::Find(_));
/// `trim` 剪
        Trim => "trim", (Wire::Trim(_));
/// `list` 列
        List => "list", (Wire::List(_));
/// `seek` 译
        Seek => "seek", (Wire::Road(_));
/// `name` 名
        Name => "name", (Wire::Name(_));
/// **`watch` 看**：订一条子树，此后树上**真变了**就往订阅者那一页里记一条事件
/// 与另七位同一条口径（一 Grant = 一枚门牌 = 一格）；它**只读**（不改树），但也不是
/// `list` 那种"问一次"——它要的是**此后**的变化，故自成一柄权
        Watch => "watch", (Wire::Watch { .. });
    }
    stem: "operator-ask-",
    wire_ty: Wire,
}
