//! program_kind — **程序装成哪种空间**（`Build` 的特权级参数：S 态页表 / U 态页表）。
//! 它是**装配表的产物**，不是程序自述：`programs::unit::PROGRAMS` 里那一行的 `space` 决定
//! （打包那一侧是 `crates/image`），引导镜像读清单后原样转交。

/// 程序装成的空间（`Build` 的特权级参数）：S 态页表 / U 态页表。
///
/// 它是**装配表的产物**，不是程序自述：`programs::unit::PROGRAMS` 里那一行的 `kind` 决定（打包那一
/// 侧是 `crates/image`），root 服务读取清单后原样转交。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProgramKind {
    /// U 态页表（页带 U 位）。
    User,
    /// S 态域（页不带 U 位，S 态 SUM=0）。
    Supervisor,
}
