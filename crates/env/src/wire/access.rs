//! **两族视图**：`Access`（对端对这份资源能做什么）与 `Policy`（这一枚能怎么流动）。
//!
//! **照实记（它们为什么住这里）**：这两个类型原先住在 `crates/runtime/src/core/port.rs`
//! ——它们由 `ship`（授出那一手）收下。但它们**只认 [`Permission`](crate::Permission)** 与
//! `core::ops`，一处也不碰内核；而 `crates/protocol` **允许依赖 `env`、不允许依赖 `runtime`**
//! ⇒ 只要它们住在 `runtime`，`driver::supply::call` 那一份（荷载的每一格都带着这两个类型）
//! 就上不了宿主靶。故搬到这里：与 `Permission` 同层、零依赖。
//!
//! **一位不多**：四位分两族（读写 / 传递），混族的值**不可表达**——`from_bits` 只收本族的位。
//! `runtime::core::port` 里 `pub use env::{Access, Policy};` 把名字照旧转出去（调用点不动）。

use core::ops::{BitAnd, BitOr, Not};

use crate::Permission;

/// 读写族：对端对这份资源**能做什么**。
///
/// 与 [`Policy`] 分成两个类型是刻意的：合成一个 `Permission` 时，调用方可以把
/// 形态位写进"读写"该在的位置，而两族相乘的十六格里有一格是空集、一格只有形态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Access(Permission);

/// 读写族那两位（解线上权时用来挡混族）。
const ACCESS_MASK: Permission = Permission::FETCH.union(Permission::STORE);
/// 传递族那两位（同上）。
const POLICY_MASK: Permission = Permission::VEST.union(Permission::ONLY);

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
