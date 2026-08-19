// 内核 panic 处理器（halt）— 输出诊断信息后停机
//
// panic 路径故意绕过所有锁：经 `console::_write` 无锁直写控制台。
use core::panic::PanicInfo;

use crate::console::_write;
use sbi::{self, fid};

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
    // 显示崩溃现场所在 hart 正在运行的任务（若有）：方便定位"哪个任务崩了"。
    // 非阻塞（try_lock）——panic 可能正发生在持有调度锁的现场，拿不到就跳过，
    // 不冒险在 panic 路径再加锁/递归。
    if let Some((tid, tname)) = crate::work::scheduler::running_task_info() {
        _write(format_args!(
            "  running task #{tid} '{tname}' (hart {})\n",
            crate::machine::hart_id()
        ));
    }
    // 格式化的 panic 消息（非字面量）也打印——诊断调试必备
    _write(format_args!("  {}\n", info.message()));

    loop {
        sbi::SystemResetCall::new(fid::SystemReset::SystemReset)
            .call()
            .unwrap();
        unsafe { core::arch::asm!("wfi") };
    }
}
