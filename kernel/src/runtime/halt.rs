// 内核 panic 处理器（halt）— 输出诊断信息后停机
//
// panic 路径故意绕过所有锁：经 `console::_write` 无锁直写控制台
// （见 lock/mod.rs 的 panic 路径说明）。
use core::panic::PanicInfo;

use crate::{
    console::_write,
    ecall::{self, fid},
};

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    _write(format_args!("[PANIC]"));
    if let Some(loc) = info.location() {
        _write(format_args!(
            " at {}:{}:{}",
            loc.file(),
            loc.line(),
            loc.column()
        ));
    }
    _write(format_args!("\n"));
    // 格式化的 panic 消息（非字面量）也打印——诊断调试必备
    _write(format_args!("  message: {}\n", info.message()));

    loop {
        ecall::SystemResetCall::new(fid::SystemReset::SystemReset)
            .call()
            .unwrap();
        unsafe { core::arch::asm!("wfi") };
    }
}
