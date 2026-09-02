//! Control 域：`ControlCall::*` 转发。

use ubi::{ControlCall, UArgs, Ucall, UcallBuilder};

pub fn panic(code: usize) -> ! {
    let args = UArgs {
        a0: code,
        ..UArgs::default()
    };
    let _ = UcallBuilder::new(Ucall::Control(ControlCall::Panic))
        .args(args)
        .call();
    unsafe { core::hint::unreachable_unchecked() }
}
