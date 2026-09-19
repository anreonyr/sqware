//! Control 域：`ControlCall::*` 转发。

use env::ControlCall;

/// 用户自诊断：采样当前任务调用栈，把 pc 地址数组写进 `buf`，返回实际帧数。
///
/// `buf` = 用户预分配的 `[usize; N]`（帧地址写入处）。内核那一边的契约是：buf 非法
/// （未映射 / 不可写）或回溯失败 ⇒ 负值（EnvError，见 `env::fid::ControlCall::Backtrace`）。
///
/// **本层把那个负值折成 `0`**（签名是 `usize`，没有第二个出口）：调用方因此分不出
/// "回溯失败"与"一帧都没采到"——要分就得改签名返回 [`env::EnvResult`]，那是另一次裁决。
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
