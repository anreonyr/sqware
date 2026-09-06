//! Control 域：`ControlCall::*` 转发。

use ubi::ControlCall;

pub fn panic(code: usize) -> ! {
    let _ = ControlCall::Panic { code }.call();
    unsafe { core::hint::unreachable_unchecked() }
}
