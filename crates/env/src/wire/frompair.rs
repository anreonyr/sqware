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
//! 故这里的纪律是：**只对"纯靠类型收窄、没有任何契约依据"的取值设防**——今天这样的
//! 取值一处也没有（全树没有 `impl FromPair for u8` 一类），故下面每一处 `from_pair`
//! 都不校验。有契约依据的取值（如 `(PieToken, Permission, TaskId)` 从 v1 取位）不设防，
//! 因为那个截断**就是**那条契约本身。
//!
//! **照实记（清掉的三处）**：本文件原先有两处 impl 自述"当前 ABI 无调用者…留着备复用"
//! ——`(usize, usize, usize, usize)`（`merge_block` 的四格计数）与 `(TaskId, TaskId)`
//! （`MailCall::Owned` 的两格打包）。还有第三处**连自述都写反了**：
//! `(TaskId, TaskId, usize)` 那条注说"`Reserve` 那条才是活的那一格"，而 `Reserve`
//! 今天标的是 `#[ret((usize, usize))]`（`a0` = owner 高 32 | vestor 低 32、`a1` = 记号），
//! 故它同样没有调用者。三处一并删：本仓对这类格子的口径是**"机制退了，格也退"**
//! （见 [`supply`](crate::wire::supply) 头注里 `Kind::Hole` 那一笔），"备复用"不在其中。

use super::{PieToken, TaskId, TeamId, VirtAddr};
use crate::HoleDir;
use crate::permission::Permission;

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

/// `Reserve` 的返回：两格**原样**交出——`a0` = owner 高 32 位 | vestor 低 32 位、
/// `a1` = **整一枚记号**（打包口径的唯一真相在 `env::fid` 的 `Reserve` 那一格的注里）。
///
/// 本层**不拆**：拆法属于调用点（`runtime::env::mail::reserve`），同一对寄存器不许有两种
/// 解释——从前这条注写的是"`(pagemeta 在手帧数, freelist 走链帧数)` 的历史遗留"，
/// 那是它换用途之前的读者，早已不成立。
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

/// `ToleCall::Await` 的返回：`v0` = 哪一枚（`0` = 没等到/没认出），`v1` = 哪个方向。
///
/// 方向只占低位两态：`0` = `Pull`、`1` = `Push`（与 `HoleDir` 的声明顺序同源）。
impl FromPair for (PieToken, HoleDir) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (
            PieToken::new(v0),
            if v1 == 1 {
                HoleDir::Push
            } else {
                HoleDir::Pull
            },
        )
    }
}

impl FromPair for (PieToken, Permission) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (PieToken::new(v0), Permission::from_bits_truncate(v1 as u32))
    }
}

/// Collect 返回值打包：v0 = token（usize），v1 低 32 位 = permission bits、v1 高 32 位 = vestor task id。
/// vestor = None 由内核编码为 `TaskId(0)`（哨兵与原 vestor=None 语义一致）。
impl FromPair for (PieToken, Permission, TaskId) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        let permission = Permission::from_bits_truncate(v1 as u32);
        let vestor = TaskId(v1 >> 32);
        (PieToken::new(v0), permission, vestor)
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
