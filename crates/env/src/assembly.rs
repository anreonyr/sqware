//! assembly — **装配单的词汇**：一行程序（[`Program`]）由哪些格子组成。
//!
//! # 为什么它住 `env`
//!
//! **同一张表要两侧读**：内核的 `build.rs`（**宿主**）按它决定"哪几台进哪张镜像"，编排域
//! （**riscv**）按它起程序。而仓里**只有 `env` 两边都编得过**——`protocol` 拖着 `runtime`
//! （那两处 riscv 内联汇编在宿主上编不过，`protocol-case` 就是为此用 `#[path]` 手工搬核心的）。
//!
//! 故这一层是"**两边都要知道的东西**"的定义处，与 [`crate::fid`]（调用号）、
//! [`crate::wire::manifest`]（清单格式）、[`crate::Permission`] / [`crate::Access`] /
//! [`crate::Policy`] / [`crate::ProgramKind`] 同款。
//!
//! **照实记（这些词是从别处搬下来的，旧路径照旧）**：`Announce` 原住
//! `protocol::system::desk`、`Grant` 原住 `programs::supervisor::system::server`、
//! `Died` 原住 `programs::supervisor::service`——三处现在都是 `pub use` 转发，**调用点一行没改**
//! （与 `Access`/`Policy` 从 `runtime::core::port` 搬到 `env::wire::access` 是同一条先例）。

use crate::{Permission, PieToken};

/// **怎么知道它起来了**——每个 Service 自己的一种，**登记时定死**。
///
/// 这一格不能一刀切：有的服务起来时会交回一条通道（那枚句柄的到达就是它的"我好了"），
/// 有的**什么都不交**（比如只走调试面的回显——它没有通道可交）。它是这一行的属性，
/// 不是调用 `start` 时的一个开关，故与名字、身子、状态同住一行。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Announce {
    /// 它会交回一枚句柄 ⇒ 那枚到了才算起来。
    Channel,
    /// 它不宣布 ⇒ **放行即起来**（"起来了"= 它没死）。
    None,
}

/// 起跑前要交出去的一枚门闩：给哪一枚、多大权。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Grant {
    /// 要交出去的那一枚（**在我表里**的句柄）。
    pub token: PieToken,
    /// 交出去的权限子集。
    pub perm: Permission,
}

/// 装配失败的编号：指"死在装配的哪一步"（沿用旧树那套小整数编号的意思）。
pub type Died = usize;
