//! call::debug — **Debug 域（class 8：借内核那支 DBCN 出口）**：调用表（[`DebugCall`]）与失败词汇（[`DebugFail`]）。

use crate::wire::VirtAddr;
use mold::{Envcall, Fail};

/// Debug 域（class 8：调试面）的失败词汇。
#[derive(Fail)]
pub enum DebugFail {
    /// `buf` 非法（未映射 / 不可读）/ 长度为零。
    Denied = -1,
}

/// `DebugFail` 的结果别名。
pub type DebugResult<T> = Result<T, erra::Error<DebugFail>>;

/// 调试调用（class 8）：域直接借内核的 DBCN。
///
/// **它不碰设备**：走的是内核自己的调试出口（SBI DBCN），故与「设备不再是内核的事」
/// 不冲突——设备写仍归持设备者，这一格只是**绕过服务**：
/// 引导期服务全都不存在，而"哪里算不下去"只有域知道。
///
/// **它不设构建门**：本仓的程序 ELF 由 `crates/image` 用**嵌套 cargo** 打包，
/// 那条内层构建与内核那一次**不共享 `debug_assertions`** ⇒ ABI 两侧的 `cfg`
/// 会分叉，域调得到一个内核不认的调用号。一个"只在某些构建里存在"的调用号是
/// **会分叉的 ABI**，代价大于它省下的那点字节。因此这一格恒在；"生产不该用它"
/// 是纪律，不是编译期的事（与 `ControlCall::Backtrace` 同一个折中）。
///
/// 本类落在 **8** 上：class 3 是空号，8 也没人占——
/// 判别号是声明顺序，取哪个空号都一样，不占任何既有号。
#[derive(Envcall)]
#[call(class = 8, fail = DebugFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DebugCall {
    /// 把域里的一段字节写进调试控制台（SBI DBCN，**不经过任何服务**）。
    ///
    /// `len == 0` / 区间未映射 ⇒ 负值；返**写出去的字节数**。
    ///
    /// **`len > DBCN_MAX` 是截断，不是错误**：内核那一步是 `len.min(DBCN_MAX)`
    /// （`envcall/debug.rs::put`），只搬前 `DBCN_MAX` 字节，返回值即搬走的长度——
    /// 要"一个字都不少"就调用方自己分段。**与 [`DebugCall::Get`] 的"多出即拒"
    /// 不同形**：那一格超长即拒，这一格超长即截。
    #[ret(usize)]
    Put { buf: VirtAddr, len: usize },
    /// 从调试控制台读一段字节写进域：内核一块栈暂存 → `Dbcn::ConsoleRead` → 写回域。
    ///
    /// **不阻塞**：没数据时**当场**返 0 字节——**不是**"等到至少读到一个字节"。
    /// 这是实测口径（OpenSBI v1.9 / QEMU virt；内核侧见
    /// `kernel/src/runtime/switcher/envcall/debug.rs::get`）。拿到 0 的调用方必须自己让一拍
    /// 或等中断，**别立刻再问**（域自己的判断，内核不替它决定）。
    ///
    /// 返**实际写进域的字节数**（`len > DBCN_MAX` ⇒ 负值，不截断）。
    #[ret(usize)]
    Get { buf: VirtAddr, len: usize },
    /// 开关**报文对账**：此后每次一次往返都把"推出去那一帧"与"收回来那一帧"的前
    /// [`DBCN_MAX`] 字节以十六进制打进调试控制台。
    ///
    /// 为什么它是 ABI 的一格而不是一个环境变量：布局错位这类病**只有真实字节能证**，
    /// "读代码推布局"无济于事。
    #[infallible]
    #[ret(())]
    SetTrace { on: usize },
}

/// [`DebugCall`] 那两格一次能搬的字节数上限。
///
/// 为什么是编译期常量：内核在入口把它拷进**栈上的定长缓冲**（诊断路径不分配，与
/// [`NOTE_MAX`] 同一条理由）。256 够一句 `file:line + 消息`，也够敲一条命令。
pub const DBCN_MAX: usize = 256;
