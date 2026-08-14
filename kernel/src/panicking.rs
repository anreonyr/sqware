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
