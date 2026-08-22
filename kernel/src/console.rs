// 控制台输出 — 内核打印 sink（legacy `sbi_console_putchar`）
//
// 命名约定：输出用 put!/putln!。
// 所有输出都经 legacy SBI console putchar 直写，无锁、无堆——panic/持锁态下也可安全调用。
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

/// 让 `fmt::Write` 的格式化器（如 table::Fmt）能把整行转发到控制台。
/// 无锁、无堆（`_write` 直写 SBI putchar）；panic/持锁态下安全。
pub struct Sink;
impl Write for Sink {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        _write(format_args!("{s}"));
        Ok(())
    }
}

/// 行首缩进包装：把多行输出整体右移 INDENT 空格——「前缀标题行顶格、其下
/// 表格/明细统一缩进」的通用实现（包住 sink 或缓冲后，render/写流时每个新行
/// 行首自动补缩进；空行不补）。无堆、无锁，panic/持锁态下安全。
pub struct Indented<W> {
    out: W,
    at_bol: bool,
}

impl<W: Write> Indented<W> {
    /// 包装一个 sink/缓冲：后续每个新行行首补缩进。
    pub fn new(out: W) -> Self {
        Self {
            out,
            at_bol: true,
        }
    }
}

impl<W: Write> Write for Indented<W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut rest = s;
        while let Some(i) = rest.find('\n') {
            if self.at_bol {
                self.out.write_str("  ")?;
                self.at_bol = false;
            }
            if i > 0 {
                self.out.write_str(&rest[..i])?;
            }
            self.out.write_char('\n')?;
            self.at_bol = true;
            rest = &rest[i + 1..];
        }
        if !rest.is_empty() {
            if self.at_bol {
                self.out.write_str("  ")?;
                self.at_bol = false;
            }
            self.out.write_str(rest)?;
        }
        Ok(())
    }
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
