// 控制台输出 — 内核打印 sink（SBI Dbcn 块写 + 段地址解析）
//
// 命名约定：输出用 put!/putln!。
//
// Dbcn 按物理地址读取：恒等区 VA 即 PA 直通；用户窗口 VA 逐段译成 PA 后写出。
use core::fmt::{self, Write};

use sbi::{DbcnCall, fid::Dbcn, scall::SArgs};

use crate::memory::manager::addr::VirtAddr;
use crate::work::room::scheduler;
use crate::work::unit::space::Space;

/// **恒等区**（DRAM 0x80000000.. dram 上界）：VA 即 PA，Dbcn 可直读。
/// 其他（用户窗口 VA）须经页表 translate。
///
/// 上界**随机器 dram 取**（[`identity_edge`]），不写死——内存容量由 DTB 决定。
const IDENTITY_BASE: usize = 0x8000_0000;

/// DRAM 恒等区上界（VA=PA 区间的 exclusive 上界）。机器信息未注入 → 退回
/// 保守 256M 上界——现场缓冲全在镜像静态区（恒在区内），取小只会让个别缓冲
/// 走丢弃分支，不误。
fn identity_edge() -> usize {
    crate::machine::dram_end().unwrap_or(0x9000_0000)
}

struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

/// 唯一 Dbcn 块写出口：a1 须为**物理**地址（OpenSBI 按物理地址读取）。PA 由
/// 调用方保证：恒等区或经译段得到。
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

/// 在指定空间上打印一段缓冲（已持空间锁的上下文用）：逐段翻译，段内 flags
/// 做 R 位检查；不重取锁。返回是否完整写出（某页未映射/不可读即中断）。
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

/// 无空间上下文打印缓冲。
///
/// 整段预判二分支，**锁只取一次**：
///   - 全段落在恒等区（内核栈/静态区）→ 无锁直通（panic 现场安全）；
///   - 否则借当前运行空间走 [`write_in`]。
///
/// 会取运行空间锁。
fn write_bytes(bytes: &[u8]) {
    let va = bytes.as_ptr() as usize;
    let end = va + bytes.len();
    if va >= IDENTITY_BASE && end <= identity_edge() {
        // 恒等区：VA 即 PA，一次直通（连续段）
        dbcn_write(va, bytes.len());
    } else if scheduler::has_running_task() {
        scheduler::with_running_space(|space| {
            write_in(space, va, bytes.len());
        });
    }
    // 无运行空间（boot/panic 早期）：用户窗口 VA 无从译 PA，静默丢弃——内核
    // 缓冲恒在恒等区（上界随 dram），此分支仅防御未来失误；宁可丢一行也不在
    // panic 现场嵌套 panic 静默挂死。
}

/// put!/putln!/log logger 的共同出口。
pub fn _write(args: fmt::Arguments) {
    let _ = Console.write_fmt(args);
}

/// 让 `fmt::Write` 的格式化器能把整行转发到控制台。
/// 无锁、无堆；panic/持锁态下安全。
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
