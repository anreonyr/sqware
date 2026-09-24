//! 门闩权限：**位掩码 + 两族视图**（用户态 + 内核态共用，单一真相）。
//!
//! 两个族，各回答一个问题：
//!   - **读写族** `FETCH | STORE`：对这份资源**能做什么**（数据面看这一族）→ [`Access`]；
//!   - **传递族** `VEST | ONLY`：这一枚**能怎么流动**（权柄面看这一族）→ [`Policy`]。
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
//! STORE    push / 等写          映射可写（PTE W）               ring          attach / detach
//! ```
//!
//! **收窄（`Narrow`）有一处资源不对称**：
//!   - Hole：任意非空子集都是合法目标（无页表约束）；
//!   - Pole：目标**必须含 `FETCH`**（RISC-V PTE 无 R=0 的合法数据叶），故
//!     `Narrow(pole, STORE)` 被拒而 `Narrow(hole, STORE)` 成功。
//!
//! 用户态用法：envcall 时 `a2 = permission.bits() as usize`。内核侧**不**做
//! `from_bits_truncate` 式的截断还原——未申明的位一律拒绝（`wire::unpack` 的
//! 拒绝式解码，见 `kernel/src/runtime/switcher/envcall/mod.rs` 的入口）。
//!
//! # 两族各有**一个视图类型**
//!
//! 上面那两个族不只是"读法的说法"：它们各有一个类型，且**混族的值不可表达**——
//! [`Access::from_bits`] 只收读写族那两位、[`Policy::from_bits`] 只收传递族那两位。
//! 一位不多。
//!
//! **照实记（`Access` / `Policy` 为什么住本文件）**：它们原先住 `runtime::core::port`
//! ——由 `ship`（授出那一手）收下；后来搬到 `env::wire::access`，理由是"只认 `Permission`
//! 与 `core::ops`、一处也不碰内核，而 `crates/protocol` 允许依赖 `env`、不允许依赖
//! `runtime`"。**但那一步只走了一半**：本文件的头注整段讲的就是这两个族，而两个族的**类型**
//! 却住在 `wire/` 那边——同一个故事分两处讲。现在合成一处：**位掩码、两族的 mask、两族的
//! 视图类型**都在本文件。
//!
//! `runtime::core::port` 里 `pub use env::{Access, Policy};` 把名字照旧转出去（调用点不动）。
//!
//! `impl Wire for Permission` 仍住 `wire/mod.rs`（与另两个"类型在外、impl 在此"的
//! `HoleDir` / `ProgramKind` 并排）：那是本仓"**非法位校验只有一处**"的落点
//! （`from_bits(...).ok_or(...)`，替代 `from_bits_truncate` 的静默截断）。

use bitflags::bitflags;
use core::ops::{BitAnd, BitOr, Not};

bitflags! {
    /// 门闩权限位掩码。
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Permission: u32 {
        /// 取用权：观察 / 接收 / 取走（`pull` / `hush` / `open` / `Await`；Pole 的映射可读）。
        const FETCH = 1 << 0;
        /// 投递权：修改 / 投递 / 写入（`push` / `ring` / `attach` / `detach`；Pole 的映射可写）。
        const STORE = 1 << 1;
        /// 目标位：把 pie 复制给其他 Task（自身 permission 不变）。
        ///
        /// **授予权 ⟺ 持本位**：它回答"这一枚能不能再流出去"（供给那一族把它读作
        /// "这一格可不可以再授出"；那个词住 `plan`，本 crate 不认识它）。
        const VEST  = 1 << 2;
        /// 形态位：这枚资源**只允许一个使用者**（授出即移交，不能复制）。
        ///
        /// 它是**资源事实**，不是调用方的选择：`Accord` 校验 `subset` 与源枚的
        /// `ONLY` 一致；`Narrow` 不得撤它。源枚带它 ⇒ 子枚必带它 ⇒ 独占链自明。
        const ONLY  = 1 << 3;
    }
}

// ── 读写族：对端对这份资源**能做什么** ────────────────────────────────────

/// 读写族那两位（解线上权时用来挡混族）。
const ACCESS_MASK: Permission = Permission::FETCH.union(Permission::STORE);

/// 读写族：对端对这份资源**能做什么**。
///
/// 与 [`Policy`] 分成两个类型是刻意的：合成一个 `Permission` 时，调用方可以把
/// 形态位写进"读写"该在的位置，而两族相乘的十六格里有一格是空集、一格只有形态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Access(Permission);

impl Access {
    /// 什么都没有：两族都取 `NONE` 才是空集（那种授予 `ship` 就地拒）。
    pub const NONE: Access = Access(Permission::empty());
    /// 取用 / 观察 / 接收（`pull` / `hush` / `open` / `Await`）。
    pub const FETCH: Access = Access(Permission::FETCH);
    /// 投递 / 修改（`push` / `ring` / `attach`）。
    pub const STORE: Access = Access(Permission::STORE);
    /// 取与投。**只为常量表而设**：`|` 不是 `const fn`，而需求单（`programs::needs`）
    /// 是编译期常量表——两族各一位，合成走这里。
    pub const FETCH_STORE: Access = Access(Permission::FETCH.union(Permission::STORE));

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
    }

    /// 从线上那几位解回（**只收本族的位**，混族与未知位一律拒）。
    ///
    /// 与"调用点写不出裸子集"是同一条：这条入口只认本来就是 `Access` 造出来的值，
    /// 故它不构成第二条授权路径，只给"过线的权"一个回来的门。
    pub const fn from_bits(bits: u32) -> Option<Access> {
        match Permission::from_bits(bits) {
            Some(p) if p.bits() & ACCESS_MASK.bits() != p.bits() => None,
            Some(p) => Some(Access(p)),
            None => None,
        }
    }
}

impl BitOr for Access {
    type Output = Access;

    fn bitor(self, rhs: Access) -> Access {
        Access(self.0 | rhs.0)
    }
}

// ── 传递族：这一枚**能怎么流动** ──────────────────────────────────────────

/// 传递族那两位（同上）。
const POLICY_MASK: Permission = Permission::VEST.union(Permission::ONLY);

/// 传递族：这一枚**能怎么流动**。
///
/// 四格读作四件事——**后两格只有独占资源上才成立**（源枚带 `ONLY` 才给得出）：
///
/// ```text
/// NONE         分  双方各持一份，对端不能再授出
/// VEST         借  双方各持一份，对端可以再授出
/// ONLY         让  我失去这一枚，对端不能再授出
/// VEST | ONLY  给  我失去这一枚，对端可以再授出
/// ```
///
/// 这四格里，**只有 `VEST` 一位是调用方的选择**：形态（复制还是移交）由**源枚**定
/// ——`ONLY` 是资源事实，`Accord` 校验 `subset` 与源枚一致，不一致 ⇒ 拒。
/// 所以想复制一枚独占资源，在这里就发不出去。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Policy(Permission);

impl Policy {
    /// 不能再授出（源枚不动）。
    pub const NONE: Policy = Policy(Permission::empty());
    /// 对端可以再授出。
    pub const VEST: Policy = Policy(Permission::VEST);
    /// 与源枚一致：独占资源的授出（这一次是**移交**，源枚在子枚存活期间不可用）。
    pub const ONLY: Policy = Policy(Permission::ONLY);

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
    }

    /// 从线上那几位解回（**只收传递族那两位**）——与 [`Access::from_bits`] 同一条口径。
    pub const fn from_bits(bits: u32) -> Option<Policy> {
        match Permission::from_bits(bits) {
            Some(p) if p.bits() & POLICY_MASK.bits() != p.bits() => None,
            Some(p) => Some(Policy(p)),
            None => None,
        }
    }
}

impl BitOr for Policy {
    type Output = Policy;

    fn bitor(self, rhs: Policy) -> Policy {
        Policy(self.0 | rhs.0)
    }
}

impl BitAnd for Policy {
    type Output = Policy;

    fn bitand(self, rhs: Policy) -> Policy {
        Policy(self.0 & rhs.0)
    }
}

/// 取反是**为"剔掉某一位"而开的口**：`policy & !Policy::VEST` 读起来就是
/// "形态照旧，只去掉'能再授出'那一位"——调用点不必为一个动作另起名词。
///
/// 取反之后的值可能带上别的族的位（如 `FETCH`），故它**只配与 `&` 联用**：
/// 单独拿它去 `ship` 就是把两族人造的值当形态发出去。
impl Not for Policy {
    type Output = Policy;

    fn not(self) -> Policy {
        Policy(!self.0)
    }
}
