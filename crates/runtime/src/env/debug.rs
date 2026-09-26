//! Debug 域（class 8）：`DebugCall::*` 转发（借内核的 DBCN 打印/读入）。
//!
//! 它们是"服务还没起来的嘴"，不是一条新的控制台通路——生产里域打印仍归 console 服务
//! （纪律，不是编译期的事：见 `env::fid::DebugCall` 那条"不设构建门"的理由）。
//!
//! 每一个函数封一次 envcall，零逻辑，与 `env::chrono`/`env::room` 同形。
//!
//! **照实记（删掉的本地那一半）**：本文件原先还有一个**域自己的**对账开关——`TRACE`
//! 静态 + `tracing()` / `set_local()` 一对读写着，外加把它打出来的 `hexdump()`。
//! 读它的那一处（`runtime::core::port::Port::call`）早已随"编帧解帧搬去各协议"而消失，
//! 故 `tracing()` 与 `hexdump()` **零调用者**（内核侧 `envcall/mod.rs` 也记着这一句：
//! "用户侧那份 `env::debug::tracing()` 因此闲置"），而 `set_local` 只为被它们读而活着
//! ——整条账一并删。**`DebugCall::SetTrace` 那一格不动**：内核那一侧的对账仍由它开关，
//! 只是域不再替自己记一份"要不要打"。

use env::{DBCN_MAX, DebugCall, DebugCallRet, EnvResult};

/// 把一个字符串写进调试控制台（`len == 0` ⇒ `Denied`；返**写出去的字节数**）。
///
/// **超 [`DBCN_MAX`] 的那一段被内核截断**（本层照 `s.len()` 递上去，不预截）：
/// 一件长东西只印出前 `DBCN_MAX` 字节，返回值就是那个数。**这不是错误**——与
/// [`get`] 的"多出即拒"不同形（两处的实测读法见 `env::fid::DebugCall`）。
pub fn put(s: &str) -> EnvResult<usize> {
    let bytes = s.as_bytes();
    let call = DebugCall::Put {
        buf: env::VirtAddr::new(bytes.as_ptr() as usize),
        len: bytes.len(),
    };
    match call.call()? {
        DebugCallRet::Put(n) => Ok(n),
        _ => unreachable!(),
    }
}

/// 从调试控制台读一段字节写进 `buf`，返**实际写入的字节数**。
///
/// **可能阻塞**（SBI 的 console read 语义）：单核上会挂住整机，直到串口来字节。
/// 敢不敢在这儿等，是调用方的判断（见 `env::fid::DebugCall::Get` 的注）。
pub fn get(buf: &mut [u8]) -> EnvResult<usize> {
    let len = buf.len().min(DBCN_MAX);
    let call = DebugCall::Get {
        buf: env::VirtAddr::new(buf.as_mut_ptr() as usize),
        len,
    };
    match call.call()? {
        DebugCallRet::Get(n) => Ok(n),
        _ => unreachable!(),
    }
}

/// 开关**报文对账**（此后每次一次往返都打两帧的十六进制）。
///
/// 为什么留在 ABI 上而不是编译期开关：布局错位只有**真实字节**能证，而这类病一旦出现
/// 就要能在**同一个产物**上打开对账再跑一遍。
///
/// **不返 `EnvResult`**：内核那一格把开关的**回读值**写进 `a0`（0/1），没有失败支
/// ——同 [`starve`](crate::env::room::starve) 的形状（见 `env::ecall::EnvResult` 的注）。
pub fn trace(on: bool) {
    let call = DebugCall::SetTrace { on: on as usize };
    let _ = call.call();
}
