//! 门闩权限位掩码（用户态 + 内核态共用，单一真相）。
//!
//! 两个族，各回答一个问题：
//!   - **读写族** `FETCH | STORE`：对这份资源**能做什么**（数据面看这一族）；
//!   - **传递族** `VEST | ONLY`：这一枚**能怎么流动**（权柄面看这一族）。
//!     - `VEST` 是**目标位**：这一枚能不能再授出（`Need::Grant` ⟺ 持 `VEST`）；
//!     - `ONLY` 是**形态位**：这枚资源**允不允许多个使用者**。它不是调用方的选择，
//!       而是**资源事实**——`Accord` 只校验 `subset` 与源枚的 `ONLY` 一致
//!       （不一致 ⇒ `Denied`）；一致时由源枚决定这一次是**移交**（源枚在子枚存活
//!       期间不可用、子枚消亡自动复原）还是**复制**（源枚照旧可用）。
//!
//! `ONLY` 只在**内核决定的地方**出现（设备 `reg` 段、组），用户态铸的门闩不带它；
//! `Narrow` 不得撤它（**自持枚也不例外**）——撤掉就等于把"只许一个使用者"洗掉。
//!
//! **位与动词**（同一枚位在四种资源上各管一件事，这正是位名必须与种类无关的原因）：
//!
//! ```text
//!          Hole                Pole                          Nole          Tole
//! FETCH    pull / 等读          open 借映 / shut / 映射可读     wait / hush   Await
//! STORE    push / 等写          映射可写（PTE W）               ring          hang / unhang
//! ```
//!
//! **收窄（`Narrow`）有一处资源不对称**：
//!   - Hole：任意非空子集都是合法目标（无页表约束）；
//!   - Pole：目标**必须含 `FETCH`**（RISC-V PTE 无 R=0 的合法数据叶），故
//!     `Narrow(pole, STORE)` 被拒而 `Narrow(hole, STORE)` 成功。
//!
//! 用户态用法：envcall 时 `a2 = permission.bits() as usize`。内核侧**不**做
//! `from_bits_truncate` 式的截断还原——未申明的位一律拒绝（`wire::unpack` 的
//! 拒绝式解码，见 `kernel/src/runtime/switcher/envcall.rs` 的入口）。

use bitflags::bitflags;

bitflags! {
    /// 门闩权限位掩码。
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Permission: u32 {
        /// 取用权：观察 / 接收 / 取走（`pull` / `hush` / `open` / `Await`；Pole 的映射可读）。
        const FETCH = 1 << 0;
        /// 投递权：修改 / 投递 / 写入（`push` / `ring` / `hang` / `unhang`；Pole 的映射可写）。
        const STORE = 1 << 1;
        /// 目标位：把 pie 复制给其他 Task（自身 permission 不变）。
        ///
        /// `Need::Grant ⟺ 持本位`：它回答"这一枚能不能再流出去"。
        const VEST  = 1 << 2;
        /// 形态位：这枚资源**只允许一个使用者**（授出即移交，不能复制）。
        ///
        /// 它是**资源事实**，不是调用方的选择：`Accord` 校验 `subset` 与源枚的
        /// `ONLY` 一致；`Narrow` 不得撤它。源枚带它 ⇒ 子枚必带它 ⇒ 独占链自明。
        const ONLY  = 1 << 3;
    }
}
