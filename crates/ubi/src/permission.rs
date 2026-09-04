//! 门闩权限位掩码（用户态 + 内核态共用）。
//!
//! v1.1: `READ | WRITE | VEST` 已实现；`BACK` 留位未实现。
//!
//! 用户态用法：envcall 时 `a2 = permission.bits() as usize`；内核侧
//! `Permission::from_bits_truncate(a2)` 还原。

use bitflags::bitflags;

bitflags! {
    /// 门闩权限位掩码。
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Permission: u32 {
        /// Read 权：观察 / 接收 / 重读。
        const READ  = 1 << 0;
        /// Write 权：修改 / 投递 / 写入。
        const WRITE = 1 << 1;
        /// Vest 权：把 pie 复制给其他 Task。
        const VEST  = 1 << 2;
        /// Back 权（预留）：只能 Vest 回 grantor。
        const BACK  = 1 << 3;
    }
}