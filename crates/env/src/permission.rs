//! 门闩权限位掩码（用户态 + 内核态共用，单一真相）。
//!
//! `READ | WRITE | VEST | BACK` 均已实现。**收窄（`Narrow`）有一处资源不对称**：
//!   - Hole：任意非空子集都是合法目标（无页表约束）；
//!   - Pole：目标**必须含 READ**（RISC-V PTE 无 R=0 的合法数据叶），故
//!     `Narrow(pole, WRITE)` 被拒而 `Narrow(hole, WRITE)` 成功。
//!
//! 用户态用法：envcall 时 `a2 = permission.bits() as usize`。内核侧**不**做
//! `from_bits_truncate` 式的截断还原——未申明的位一律拒绝（`wire::unpack` 的
//! 拒绝式解码，见 `kernel/src/runtime/switcher/envcall.rs` 的入口）。

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
