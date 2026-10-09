//! 返回值蒸馏——内核回写的寄存器 → 域 Ret 载荷。
//!
//! 每个 `#[ret(T)]` 的 `T` 在此实现 [`FromPair`]；derive 生成的 `call()`
//! 在非负路径调用 `<T as FromPair>::from_pair(v0, v1)`。错误路径由
//! 各域的**词表读法**（生成的入口里那一格）接管，故此处只见成功值。
//!
//! **宽返回那一格**（`#[ret3(T)]`，`MailCall::Pull` 与 `MailCall::Peek`）走
//! [`FromTriple`]：两格寄存器装不下它那几件事实，故读 `a0..a2`。两条路的分工是**线宽**，
//! 不是语义——能用一对说完的仍走 [`FromPair`]（三十格），别为了省一次改写把两件事挤进一格。
//!
//! # 本文件的校验口径（与 [`Wire`](crate::wire::Wire) 的分工）
//!
//! 两者**不是同一类东西**，故口径**本来就该不同**——但不同不等于不设防：
//!
//! | | [`Wire::unpack`](crate::wire::Wire::unpack) | 本文件 `from_pair` |
//! |---|---|---|
//! | 数据来自 | **用户态**（a0..a5，完全可控） | **内核回写**（a0/a1；宽那一格 a0..a2） |
//! | 策略 | **拒绝**：非法位 / 超宽 → `Err(Decode)` | **按契约取位**（截断是位打包的一部分） |
//! | 依据 | 不可信输入 | §"由内核保证" |
//!
//! 内核侧那一条不写成 `Err`，理由是**错误域的形状**：各域词表（`abi/call` 那一份）的
//! D1 契约）说的都是**内核→用户**的答案（`Denied`/`Dead`/`Busy`/…）。把"内核自己违约"
//! 塞进同一个域，等于让用户程序去处理内核的 bug，且每条 `call()` 都要多一个分支。
//!
//! 故这里的纪律是：**只对"纯靠类型收窄、没有任何契约依据"的取值设防**——今天这样的
//! 取值一处也没有（全树没有 `impl FromPair for u8` 一类），故下面每一处 `from_pair`
//! 都不校验。有契约依据的取值（如 `(PieToken, Permission)` 从 v1 取位）不设防，
//! 因为那个截断**就是**那条契约本身。

use super::{PieToken, TaskId, TeamId, VirtAddr};
use crate::abi::permission::Permission;

/// 由内核回写的 `(a0, a1)` 还原「域 Ret 载荷」的契约（R3 蒸馏）。
///
/// derive(Envcall) 生成的 `call()` 在正（非负）路径按 variant 调用
/// `<T as FromPair>::from_pair(v0, v1)` 组装 Ret 载荷；错误路径已在
/// 词表读法那一分支，故此处只见成功值。
pub trait FromPair: Sized {
    fn from_pair(v0: usize, v1: usize) -> Self;
}

/// 由内核回写的 `(a0, a1, a2)` 还原**那一格宽载荷**的契约（R3 蒸馏）。
///
/// 与 [`FromPair`] 只差**线宽**：一对寄存器说不完的返回走这一条。derive 生成的
/// `call()` 对 `#[ret3(T)]` 那几个 variant 调 `<T as FromTriple>::from_triple(v0, v1, v2)`；
/// **同一枚枚举里两条路可以并存**（Mail 的调用表同时使用两种宽度），按 variant 各走各的。
///
/// **为什么不是"给 [`FromPair`] 多加两个参数"**：那会让三十格只读 `a0`/`a1` 的
/// 返回各自多背两个空位，而这一格只有一处用家。宽窄是**这一格自己的事实**，
/// 故写在它自己的 impl 上。
pub trait FromTriple: Sized {
    fn from_triple(v0: usize, v1: usize, v2: usize) -> Self;
}

impl FromPair for () {
    fn from_pair(_v0: usize, _v1: usize) -> Self {}
}

impl FromPair for usize {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0
    }
}

/// `Pull` 的返回：`(实际长度, 发送者 task id)`。
impl FromPair for (usize, TaskId) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (v0, TaskId(v1))
    }
}

/// Two whole return words.
impl FromPair for (usize, usize) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (v0, v1)
    }
}

/// `Open` 的返回：`(视图起点, 整段多大)`——一段区间的两半，故一起回。
impl FromPair for (VirtAddr, usize) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (VirtAddr(v0), v1)
    }
}

impl FromPair for u64 {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0 as u64
    }
}

impl FromPair for (u64, u64) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (v0 as u64, v1 as u64)
    }
}

impl FromPair for (PieToken, PieToken) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (PieToken::new(v0), PieToken::new(v1))
    }
}

impl FromPair for (PieToken, Permission) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (PieToken::new(v0), Permission::from_bits_truncate(v1 as u32))
    }
}

/// `Peek` 的返回：`(长度, 发送者, 队里排着几只)`。
///
/// 第三格是**队列深度**——写者据此知道"我还排着几手"（孔上可以排着至多 `QUEUE_CAP` 只手）。
/// 三件事实两格装不下，故与 `Collect` 走同一条宽返回的路（`#[ret3]`）。
impl FromTriple for (usize, TaskId, usize) {
    fn from_triple(v0: usize, v1: usize, v2: usize) -> Self {
        (v0, TaskId(v1), v2)
    }
}

impl FromPair for bool {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0 != 0
    }
}

impl FromPair for PieToken {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        PieToken::new(v0)
    }
}

impl FromPair for TaskId {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        TaskId(v0)
    }
}

impl FromPair for TeamId {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        TeamId(v0)
    }
}

impl FromPair for VirtAddr {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        VirtAddr(v0)
    }
}

impl FromPair for (PieToken, crate::MailCondition) {
    fn from_pair(token: usize, condition: usize) -> Self {
        (
            PieToken::new(token),
            crate::MailCondition::of(condition).expect("invalid Await condition"),
        )
    }
}
