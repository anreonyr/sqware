// 控制台输出 — 内核打印 sink（legacy `sbi_console_putchar`）
//
// 命名约定：输出用 put!/putln!，未来输入侧对应 get!/getln!（本文件暂不实现）。
// 所有输出都经 legacy SBI console putchar 直写，无锁、无堆——panic/持锁态下也可安全调用。
// （QEMU virt 未配 debug console，DBCN `sbi_debug_console_write` 是空操作，故用 legacy。）
use core::fmt::{self, Write};

use sbi::{DbcnCall, fid::Dbcn, scall::SArgs};

struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

fn write_bytes(bytes: &[u8]) {
    DbcnCall::new(Dbcn::ConsoleWrite)
        .args(SArgs {
            a0: bytes.len(),
            a1: bytes.as_ptr().addr(),
            ..Default::default()
        })
        .call()
        .unwrap();
}

/// put!/putln!/log logger 的共同出口。
pub fn _write(args: fmt::Arguments) {
    let _ = Console.write_fmt(args);
}

#[macro_export]
macro_rules! put {
    ($($arg:tt)*) => { $crate::console::_write(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! putln {
    () => { $crate::put!("\n") };
    ($($arg:tt)*) => { $crate::console::_write(format_args!("{}\n", format_args!($($arg)*))) };
}

// 未来输入侧：get!/getln!（预留，本阶段不实现）

// ── log crate 集成 ──────────────────────────────

struct KernelLogger;
static LOGGER: KernelLogger = KernelLogger;

impl log::Log for KernelLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            _write(format_args!("[{}] {}\n", record.level(), record.args()));
        }
    }

    fn flush(&self) {}
}

/// 注册 log crate 全局 logger（恰好一次，任何 log::* 之前调用）。
pub fn init() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}
