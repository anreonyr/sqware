use ::core::fmt;

use sbi::{DbcnCall, ecall::SArgs, fid::Dbcn};

use crate::core::memory::address::VirtAddr;

mod line;
mod logger;
mod sink;

use line::Line;
pub use logger::init;
pub use sink::Sink;

const IDENTITY_BASE: usize = 0x8000_0000;

fn identity_edge() -> usize {
    crate::machine::board::dram_edge().unwrap_or(0x9000_0000)
}

fn write(s: &str) {
    let bytes = s.as_bytes();
    if let Some(pa) = physical_buffer(bytes.as_ptr() as usize, bytes.len()) {
        DbcnCall::new(Dbcn::ConsoleWrite)
            .args(SArgs {
                a0: bytes.len(),
                a1: pa,
                ..Default::default()
            })
            .call()
            .expect("Dbcn");
    }
}

pub fn read(buf: &mut [u8]) -> Option<usize> {
    if buf.is_empty() {
        return None;
    }
    let pa = physical_buffer(buf.as_ptr() as usize, buf.len())?;
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

fn physical_buffer(va: usize, len: usize) -> Option<usize> {
    if va >= IDENTITY_BASE && va + len <= identity_edge() {
        Some(va)
    } else {
        translate_kernel(va, len)
    }
}

fn translate_kernel(va: usize, len: usize) -> Option<usize> {
    if VirtAddr::from_raw(va).is_user() {
        return None;
    }
    let space = &crate::core::work::unit::team::kernel()?.space;
    let pa0 = space.memory.physical_at(VirtAddr::from_raw(va))?;
    let mut va_cur = va;
    let end = va + len;
    while va_cur < end {
        let pa = space.memory.physical_at(VirtAddr::from_raw(va_cur))?;
        if pa.as_usize() != pa0.as_usize() + (va_cur - va) {
            return None;
        }
        va_cur = (va_cur & !(crate::core::memory::PAGE_SIZE - 1)) + crate::core::memory::PAGE_SIZE;
    }
    Some(pa0.as_usize())
}

pub fn _write(args: fmt::Arguments) {
    let mut line = Line::new();
    let _ = fmt::write(&mut line, args);
    line.emit();
}

#[macro_export]
macro_rules! put {
    ($($arg:tt)*) => { $crate::machine::console::_write(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! putln {
    () => { $crate::put!("\n") };
    ($($arg:tt)*) => { $crate::machine::console::_write(format_args!("{}\n", format_args!($($arg)*))) };
}
