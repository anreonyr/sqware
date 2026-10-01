//! Debug 域（class 8）：`DebugCall::*` 转发（借内核的 DBCN 打印/读入）。
//!
//! 它们是"服务还没起来的嘴"，不是一条新的控制台通路——生产里域打印仍归 console 服务
//! （纪律，不是编译期的事：见 `env::abi::call::DebugCall` 那条"不设构建门"的理由）。
//!
//! 每一个函数封一次 envcall，零逻辑，与 `env::chrono`/`env::room` 同形。

use env::{DBCN_MAX, DebugResult, VirtAddr};

/// 把一个字符串写进调试控制台（`len == 0` ⇒ `Denied`；返**写出去的字节数**）。
///
/// **超 [`DBCN_MAX`] 的那一段被内核截断**（本层照 `s.len()` 递上去，不预截）：
/// 一件长东西只印出前 `DBCN_MAX` 字节，返回值就是那个数。**这不是错误**——与
/// [`get`] 的"多出即拒"不同形（两处的实测读法见 `env::abi::call::DebugCall`）。
///
/// # Errors
/// - `Denied`(-1) `buf` 未映射 / 长度为零
pub fn put(s: &str) -> DebugResult<usize> {
    let bytes = s.as_bytes();
    env::debug::put(VirtAddr::new(bytes.as_ptr() as usize), bytes.len())
}

/// 从调试控制台读一段字节写进 `buf`，返**实际写入的字节数**。
///
/// **可能阻塞**（SBI 的 console read 语义）：单核上会挂住整机，直到串口来字节。
/// 敢不敢在这儿等，是调用方的判断（见 `env::abi::call::DebugCall::Get` 的注）。
///
/// # Errors
/// - `Denied`(-1) `buf` 非法 / 长度为零或超 `DBCN_MAX` / 固件给不出这一格
pub fn get(buf: &mut [u8]) -> DebugResult<usize> {
    let len = buf.len().min(DBCN_MAX);
    env::debug::get(VirtAddr::new(buf.as_mut_ptr() as usize), len)
}

/// 开关**报文对账**（此后每次一次往返都打两帧的十六进制）。
///
/// 为什么留在 ABI 上而不是编译期开关：布局错位只有**真实字节**能证，而这类病一旦出现
/// 就要能在**同一个产物**上打开对账再跑一遍。
///
/// **这一格不失败**（`#[infallible]`）：内核把开关的**回读值**写进 `a0`（0/1），没有失败支
/// ⇒ 生成的入口 `env::debug::set_trace` 返 `()`，本层照旧把它当"开关一手"用。
pub fn trace(on: bool) {
    env::debug::set_trace(on as usize)
}
