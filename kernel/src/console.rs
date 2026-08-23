// 控制台输出 — 内核打印 sink（SBI Dbcn 块写 + 段地址解析）
//
// 命名约定：输出用 put!/putln!。
//
// **缓冲地址必须转成 Dbcn 可读的物理地址**：Dbcn::ConsoleWrite 的 a1 由 OpenSBI
// 在 M-mode 按**物理**地址读取。内核打印缓冲分两类——
//   恒等区（boot/trap 栈、静态区、panic 现场）→ VA 即 PA，直通；
//   用户窗口 VA（任务栈上的 Fmt 等，如 ktask 闭包打印）→ 经 `Space::segments`
//   迭代器逐段译成 PA（VA 连续 ≠ 物理帧连续，segments 按页切段，段长即页内
//   余量，不跨物理帧）。
//   曾因把 0xC0000000 用户窗口 VA 直传 Dbcn，OpenSBI M-mode 读无物理内存 →
//   load fault → 固件 console 锁死（全系统静默卡死的根因）。
//
// 段来源两路、出口唯一：`dbcn_write` 是唯一 Dbcn 块写出口；`write_in`
// （envcall Write 持锁上下文，借 running space 译段）与 `write_bytes`
// （无空间上下文兜底：恒等直通 / with_running_space 转发）都消费它。
use core::fmt::{self, Write};

use sbi::{DbcnCall, fid::Dbcn, scall::SArgs};

use crate::memory::manager::addr::VirtAddr;
use crate::work::room::scheduler;
use crate::work::unit::space::Space;

/// **恒等区**（DRAM 0x80000000..0x90000000，同 scene::walk_sv39 的守卫范围）：
/// VA 即 PA，Dbcn 可直读。其他（用户窗口 VA）须经页表 translate。
const IDENTITY_BASE: usize = 0x8000_0000;
const IDENTITY_EDGE: usize = 0x9000_0000;

struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

/// 唯一 Dbcn 块写出口：a1 = **物理**地址（OpenSBI M-mode 直读，绝不可传 VA——
/// 曾把用户窗口 VA 直传致固件 load fault。PA 由调用方保证：恒等区或
/// `segments` 译出）。
fn dbcn_write(pa: usize, len: usize) {
    DbcnCall::new(Dbcn::ConsoleWrite)
        .args(SArgs {
            a0: len,
            a1: pa,
            ..Default::default()
        })
        .call()
        .unwrap();
}

/// 在指定空间上打印一段缓冲（**envcall Write / 已持空间锁上下文用**）：
/// 借 space 经 `segments` 逐段翻译，段内 flags 做 R 位检查；不重取锁。
/// 返回是否完整写出（某页未映射/不可读即中断，调用方据此回错误码）。
pub(crate) fn write_in(space: &Space, va: usize, len: usize) -> bool {
    let mut full = true;
    for (pa, flags, l) in space.segments(VirtAddr::from_raw(va), len) {
        if !flags.intersects(crate::memory::manager::entry::PteFlags::R) {
            full = false;
            break;
        }
        dbcn_write(pa.as_usize(), l);
    }
    full
}

/// 无空间上下文打印缓冲（banner / ktask / panic 兜底）。
///
/// 整段预判二分支，**锁只取一次**：
///   - 全段落在恒等区（内核栈/静态区）→ 无锁直通（panic 现场安全）；
///   - 否则经 `with_running_space` 借当前空间走 [`write_in`]。
///
/// **调用方不得已持调度/空间锁**（本函数会取 running space 锁）——envcall
/// Write 走 [`write_in`]（调用方已持锁），不调本函数。
fn write_bytes(bytes: &[u8]) {
    let va = bytes.as_ptr() as usize;
    let end = va + bytes.len();
    if va >= IDENTITY_BASE && end <= IDENTITY_EDGE {
        // 恒等区：VA 即 PA，一次直通（连续段）
        dbcn_write(va, bytes.len());
    } else {
        scheduler::with_running_space(|space| {
            write_in(space, va, bytes.len());
        });
    }
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
    log::set_max_level(log::LevelFilter::Debug);
}
