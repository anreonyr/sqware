//! 一份来源页在全部关系上的裁决。

/// 枚举压在这份来源上的全部关系之后的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Clearance {
    /// 没有关系还压着这一页。
    Clear,
    /// 有关系还握着这一页，或它的地址空间还有未确认的失效。
    Blocked,
    /// 有关系压着这一页，但认住它的地址空间已失联：旧 TLB 无从核实。
    Unreachable,
}
