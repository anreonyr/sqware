//! Control 域：`ControlCall::*` 转发。

use env::ControlCall;

/// 用户自诊断：采样当前任务调用栈，把 pc 地址数组写进 `buf`，返回实际帧数。
///
/// `buf` = 用户预分配的 `[usize; N]`（帧地址写入处）；返回负数（EnvError）当 buf
/// 非法（未映射/不可写）或回溯失败。
pub fn backtrace(buf: &mut [usize]) -> usize {
    let n = ControlCall::Backtrace {
        buf: buf.as_mut_ptr() as usize,
        frames: buf.len(),
    }
    .call();
    match n {
        Ok(env::ControlCallRet::Backtrace(count)) => count,
        _ => 0,
    }
}
