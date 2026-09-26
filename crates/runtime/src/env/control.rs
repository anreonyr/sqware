//! Control 域：`ControlCall::*` 转发。

/// 用户自诊断：采样当前任务调用栈，把 pc 地址数组写进 `buf`，返回实际帧数。
///
/// `buf` = 用户预分配的 `[usize; N]`（帧地址写入处）。内核那一边的契约是：buf 非法
/// （未映射 / 不可写）⇒ `ControlFail::Denied`（见 `env::fid::ControlCall::Backtrace`）。
///
/// **本层把那一枚失败折成 `0`**（签名是 `usize`，没有第二个出口）：调用方因此分不出
/// "回溯失败"与"一帧都没采到"。这一格是**本层自己的政策**——要分就改签名返回
/// [`env::ControlResult`]，那是另一次裁决（域词汇已经备好）。
pub fn backtrace(buf: &mut [usize]) -> usize {
    match env::control::backtrace(buf.as_mut_ptr() as usize, buf.len()) {
        Ok(count) => count,
        Err(_) => 0,
    }
}
