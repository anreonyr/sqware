//! 返回值蒸馏——内核回写的 `(a0, a1)` → 域 Ret 载荷。
//!
//! 每个 `#[ret(T)]` 的 `T` 在此实现 [`FromPair`]；derive 生成的 `call()`
//! 在非负路径调用 `<T as FromPair>::from_pair(v0, v1)`。错误路径由
//! `EnvError::from_raw` 接管，故此处只见成功值。
//!
//! # 本文件的校验口径（与 [`Wire`](crate::wire::Wire) 的分工）
//!
//! 两者**不是同一类东西**，故口径**本来就该不同**——但不同不等于不设防：
//!
//! | | [`Wire::unpack`](crate::wire::Wire::unpack) | 本文件 `from_pair` |
//! |---|---|---|
//! | 数据来自 | **用户态**（a0..a5，完全可控） | **内核回写**（a0/a1，契约约束） |
//! | 策略 | **拒绝**：非法位 / 超宽 → `Err(Decode)` | **按契约取位**（截断是位打包的一部分） |
//! | 依据 | 不可信输入 | §"由内核保证" |
//!
//! 内核侧那一条不写成 `Err`，理由是**错误域的形状**：`EnvError` 整张表（`ecall.rs` 的
//! D1 契约）说的都是**内核→用户**的答案（`Denied`/`Dead`/`Busy`/…）。把"内核自己违约"
//! 塞进同一个域，等于让用户程序去处理内核的 bug，且每条 `call()` 都要多一个分支。
//!
//! 故这里的纪律是：**只对"纯靠类型收窄、没有任何契约依据"的取值设防**——今天只有一处
//! （`u8`，见下），并在 `debug_assertions` 档（门的 harden 轮）**真的查**。
//! 有契约依据的取值（如 `(PieToken, Permission, TaskId)` 从 v1 取位）不设防，
//! 因为那个截断**就是**那条契约本身。

use super::{PieToken, TaskId, TeamId, VirtAddr};

/// 由内核回写的 `(a0, a1)` 还原「域 Ret 载荷」的契约（R3 蒸馏）。
///
/// derive(Envcall) 生成的 `call()` 在正（非负）路径按 variant 调用
/// `<T as FromPair>::from_pair(v0, v1)` 组装 Ret 载荷；错误路径已在
/// `EnvError::from_raw` 分支，故此处只见成功值。
pub trait FromPair: Sized {
    fn from_pair(v0: usize, v1: usize) -> Self;
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
        (PieToken(v0), PieToken(v1))
    }
}

impl FromPair for (PieToken, crate::permission::Permission) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (
            PieToken(v0),
            crate::permission::Permission::from_bits_truncate(v1 as u32),
        )
    }
}

/// Collect 返回值打包：v0 = token（usize），v1 低 32 位 = permission bits、v1 高 32 位 = vestor task id。
/// vestor = None 由内核编码为 `TaskId(0)`（哨兵与原 vestor=None 语义一致）。
impl FromPair for (PieToken, crate::permission::Permission, TaskId) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        let permission = crate::permission::Permission::from_bits_truncate(v1 as u32);
        let vestor = TaskId(v1 >> 32);
        (PieToken(v0), permission, vestor)
    }
}

/// Owned 返回值打包：v0 = vestor（授与人）、v1 = owner（资源开辟者）。
impl FromPair for (TaskId, TaskId) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (TaskId(v0), TaskId(v1))
    }
}

impl FromPair for bool {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0 != 0
    }
}

/// `u8` 是**唯一**一处"宽值 → 窄类型"的收窄：内核侧 `console::pull()` 给的是 `u8`
/// （`envcall.rs` 的 `Some(b) => b as usize`），故 a0 恒 ≤ 255——但那个保证**只由类型
/// 承载，没有任何契约或位打包为它背书**（对比：`(PieToken, Permission, TaskId)` 的
/// `as u32` 取位是契约本身）。所以这一处设防。
///
/// release 档是 `as u8`（零开销）；`debug_assertions` 档（门的 harden 轮）多一次比较，
/// 内核违约时当场点位到这一行，而不是让用户程序拿到一个与自己写入无关的字节。
impl FromPair for u8 {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        debug_assert!(
            v0 <= u8::MAX as usize,
            "IOCall::Get 的返回值必须是一字节（a0 ∈ 0..=255）：收窄无契约依据，越界即内核违约"
        );
        v0 as u8
    }
}

impl FromPair for PieToken {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        PieToken(v0)
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
