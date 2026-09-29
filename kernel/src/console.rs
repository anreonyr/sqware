use core::fmt::{self, Write};

use sbi::{DbcnCall, ecall::SArgs, fid::Dbcn};

use crate::memory::manager::addr::VirtAddr;

const IDENTITY_BASE: usize = 0x8000_0000;

fn identity_edge() -> usize {
    crate::platform::machine::dram_edge().unwrap_or(0x9000_0000)
}

struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let va = bytes.as_ptr() as usize;
        let end = va + bytes.len();
        if va >= IDENTITY_BASE && end <= identity_edge() {
            DbcnCall::new(Dbcn::ConsoleWrite)
                .args(SArgs {
                    a0: bytes.len(),
                    a1: va,
                    ..Default::default()
                })
                .call()
                .expect("Dbcn");
        } else if let Some(pa) = translate_kernel(va, bytes.len()) {
            DbcnCall::new(Dbcn::ConsoleWrite)
                .args(SArgs {
                    a0: bytes.len(),
                    a1: pa,
                    ..Default::default()
                })
                .call()
                .expect("Dbcn");
        }
        Ok(())
    }
}

pub fn read(buf: &mut [u8]) -> Option<usize> {
    if buf.is_empty() {
        return None;
    }
    let va = buf.as_ptr() as usize;
    let end = va + buf.len();
    let pa = if va >= IDENTITY_BASE && end <= identity_edge() {
        va
    } else {
        translate_kernel(va, buf.len())?
    };
    // SAFETY: pa 指向可写、物理连续的缓冲
    let got = DbcnCall::new(Dbcn::ConsoleRead)
        .args(SArgs {
            a0: buf.len(),
            a1: pa,
            ..Default::default()
        })
        .call()
        .ok()?;
    Some(got.min(buf.len()))
}

fn translate_kernel(va: usize, len: usize) -> Option<usize> {
    if VirtAddr::from_raw(va).is_user() {
        return None;
    }
    let space = &crate::work::unit::team::kernel()?.space;
    let (pa0, _) = space.translate(VirtAddr::from_raw(va))?;
    let mut va_cur = va;
    let end = va + len;
    while va_cur < end {
        let (pa, _) = space.translate(VirtAddr::from_raw(va_cur))?;
        if pa.as_usize() != pa0.as_usize() + (va_cur - va) {
            return None;
        }
        va_cur = (va_cur & !(crate::memory::PAGE_SIZE - 1)) + crate::memory::PAGE_SIZE;
    }
    Some(pa0.as_usize())
}

const LINE_MAX: usize = 256;

struct Line {
    buf: [u8; LINE_MAX],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Line {
            buf: [0u8; LINE_MAX],
            len: 0,
        }
    }

    fn emit(&mut self) {
        if self.len == 0 {
            return;
        }
        if let Ok(text) = core::str::from_utf8(&self.buf[..self.len]) {
            let _ = Console.write_str(text);
        }
        self.len = 0;
    }
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if self.buf.len() - self.len >= s.len() {
            self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
            self.len += s.len();
        } else {
            self.emit();
            let _ = Console.write_str(s);
        }
        Ok(())
    }
}

pub fn _write(args: fmt::Arguments) {
    let mut line = Line::new();
    let _ = fmt::write(&mut line, args);
    line.emit();
}

pub struct Sink;
impl Write for Sink {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        _write(format_args!("{s}"));
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

pub fn init() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Debug);
}
