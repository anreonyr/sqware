//! 门闩权限位掩码（用户态 + 内核态共用，单一真相）。
//!
//! 两个族，各回答一个问题：
//!   - **读写族** `READ | WRITE`：对这份资源能做什么（数据面看这一族）；
//!   - **传递族** `VEST | CAGE`：这一枚能怎么流动（权柄面看这一族）。
//!     - `VEST` 是**目标位**：收方能再传（任意目标）；
//!     - `CAGE` 是**形态位**：这一枚是**被关住的**——一次交出，授出方在交出期间
//!       不可用它（源枚在借）。
//!
//! `Need::Grant` ⟺ 持 `VEST`：**`CAGE` 不授予任何事，它是一条声明**。声明随副本一起
//! 被造出来，且只能收紧不能放宽（`covers` / `Narrow`）：从带 `CAGE` 的枚授出时，子枚
//! 必带 `CAGE`（粘性），`Narrow` 也不得撤它——否则独占链会被洗掉。
//!
//! 源枚上的 `CAGE` 读作"我有资格交出去"（由 `covers` 保证：`subset ⊆ 自身`）；
//! 子枚上的 `CAGE` 读作"这一枚是被交出来的"。两处读同一个位。
//!
//! **收窄（`Narrow`）有一处资源不对称**：
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
        /// 目标位：把 pie 复制给其他 Task（自身 permission 不变）。
        ///
        /// `Need::Grant ⟺ 持本位`：它回答"这一枚能不能再流出去"。
        const VEST  = 1 << 2;
        /// 形态位：这一枚是**被关住的**（一次交出；授出方在交出期间不可用它）。
        ///
        /// 授予方把本位写进 `subset`，子枚由此携带"这是一次交出"的声明：
        /// 子枚存在 ⇒ 源枚不可用（数据面答 `Caged`）；子枚消亡 ⇒ 源枚自动复原。
        /// 复原不是谁的动作，是判据读出来的事实——不需要任何通知。
        const CAGE  = 1 << 3;
    }
}
