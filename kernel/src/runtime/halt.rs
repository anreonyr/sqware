// 内核 panic 处理器（halt）— 输出诊断信息后停机
//
// panic 路径故意绕过所有锁：经 `console::_write` 无锁直写控制台
// （见 lock/mod.rs 的 panic 路径说明）。
use core::panic::PanicInfo;

use crate::console::_write;

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    _write(format_args!("[KERNEL PANIC]"));
    if let Some(loc) = info.location() {
        _write(format_args!(
            " at {}:{}:{}",
            loc.file(),
            loc.line(),
            loc.column()
        ));
    }
    _write(format_args!("\n"));
    if let Some(msg) = info.message().as_str() {
        _write(format_args!("  message: {msg}\n"));
    }

    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}
