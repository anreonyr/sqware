//! 门闩权限位掩码（用户态 + 内核态共用，单一真相）。
//!
//! `READ | WRITE | VEST | BACK` 均已实现。注意 RESTRICT 的资源不对称：
//!   - Hole：任意非空子集都可作 restrict 目标（无页表约束）；
//!   - Pole：restrict 目标**必须含 READ**（RISC-V PTE 无 R=0 合法数据叶子），
//!     因此 `restrict(pole, WRITE)` 返回 Denied 而 `restrict(hole, WRITE)` 成功。
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
        /// Vest 权：把 pie 复制给其他 Task（自身 permission 不变）。
        const VEST  = 1 << 2;
        /// Back 权：只能 Vest 回 grantor（`vestor`）。
        ///
        /// 独立限制位：只要 pie 带 BACK，vest 目标就恒被守门为 `target == vestor`
        /// ——即便同时带 VEST 位也取最严（BACK 压制 VEST 的自由性）。
        const BACK  = 1 << 3;
    }
}
