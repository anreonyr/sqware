//! 返回值蒸馏——内核回写的 `(a0, a1)` → 域 Ret 载荷。
//!
//! 每个 `#[ret(T)]` 的 `T` 在此实现 [`FromPair`]；derive 生成的 `call()`
//! 在非负路径调用 `<T as FromPair>::from_pair(v0, v1)`。错误路径由
//! `EnvError::from_raw` 接管，故此处只见成功值。

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
        let vestor = TaskId((v1 >> 32) as usize);
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

impl FromPair for u8 {
    fn from_pair(v0: usize, _v1: usize) -> Self {
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
