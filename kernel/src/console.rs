// 控制台输出 — 内核打印 sink（SBI Dbcn 块写 + 段地址解析）
//
// 命名约定：输出用 put!/putln!。
//
// Dbcn 按物理地址读取：恒等区 VA 即 PA 直通；用户窗口 VA 逐段译成 PA 后写出。
use core::fmt::{self, Write};

use sbi::{DbcnCall, fid::Dbcn, scall::SArgs};

use crate::memory::manager::addr::VirtAddr;
use crate::work::room::conductor::core::ident;
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
    crate::machine::dram_edge().unwrap_or(0x9000_0000)
}

struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let va = bytes.as_ptr() as usize;
        let end = va + bytes.len();
        if va >= IDENTITY_BASE && end <= identity_edge() {
            // 恒等区：VA 即 PA，一次直通（连续段）
            DbcnCall::new(Dbcn::ConsoleWrite)
                .args(SArgs {
                    a0: bytes.len(),
                    a1: va,
                    ..Default::default()
                })
                .call()
                .expect("Dbcn");
        } else if let Some(pa) = translate_kernel(va, bytes.len()) {
            // 内核空间映射（高半区 trap 栈 / 内核堆 / 镜像恒等区外物理帧）：
            // 无锁 walk 页表树——关机审计（trap 栈，任务全退 ident=Last）与
            // panic 现场（其他核已停）都依赖这条路径。
            DbcnCall::new(Dbcn::ConsoleWrite)
                .args(SArgs {
                    a0: bytes.len(),
                    a1: pa,
                    ..Default::default()
                })
                .call()
                .expect("Dbcn");
        } else if let Some(info) = ident()
            && let Some(task) = info.live()
        {
            // 当前任务身份槽：一次读（无锁）。Live 才有正在用的地址空间可翻译
            // （boot/空闲/末次身份 → 静默丢弃——写用户缓冲只发生在任务上下文）。
            push(&task.team.space, va, bytes.len());
        }
        Ok(())
    }
}

/// 内核空间无锁翻译 `[va, va+len)`：整段须在同一物理连续块内（每页 walk 检查
/// 连续性），否则 None。内核空间构造后页表树只读，`translate_unlocked` 安全。
/// 用于非恒等区、且无活任务身份可译的高半区内核地址（trap 栈 / 内核堆缓冲）。
///
/// **仅限内核半区地址**：用户半区 VA（如用户堆 0x87xxxxxx）在内核空间虽也有
/// identity 映射，但用户空间的该 VA 映射到**不同的物理帧**（独立分配）——用
/// 内核空间翻译会得到错误的 PA。用户地址一律回退活任务空间路径（`push`）。
fn translate_kernel(va: usize, len: usize) -> Option<usize> {
    // 用户半区地址：内核空间翻译无意义（见 doc 注释），直接回退。
    if VirtAddr::from_raw(va).is_user() {
        return None;
    }
    let space = &crate::work::unit::team::kernel()?.space;
    // SAFETY: 内核空间装配后只读；诊断路径不持 Space 锁（多核 panic 现场其他核
    // 已停，持锁会死锁）。
    let (pa0, _) = unsafe { space.translate_unlocked(VirtAddr::from_raw(va)) }?;
    // 逐页校验物理连续性：物理帧可能不连续，须整段连续方可一次 Dbcn 直读。
    let mut va_cur = va;
    let end = va + len;
    while va_cur < end {
        // SAFETY: 同上。
        let (pa, _) = unsafe { space.translate_unlocked(VirtAddr::from_raw(va_cur)) }?;
        if pa.as_usize() != pa0.as_usize() + (va_cur - va) {
            return None; // 物理不连续：退回静默（不逐段拼）
        }
        // 跳到下一页（跨过本页剩余部分）
        va_cur = (va_cur & !(crate::memory::PAGE_SIZE - 1)) + crate::memory::PAGE_SIZE;
    }
    Some(pa0.as_usize())
}

/// 在指定空间上打印一段缓冲（已持空间锁的上下文用）：逐段翻译，段内 flags
/// 做 R 位检查；不重取锁。返回是否完整写出（某页未映射/不可读即中断）。
pub(crate) fn push(space: &Space, va: usize, len: usize) -> bool {
    let mut full = true;
    for (pa, flags, l) in space.segments(VirtAddr::from_raw(va), len) {
        if !flags.intersects(crate::memory::manager::entry::PteFlags::R) {
            full = false;
            break;
        }
        DbcnCall::new(Dbcn::ConsoleWrite)
            .args(SArgs {
                a0: l,
                a1: pa.as_usize(),
                ..Default::default()
            })
            .call()
            .unwrap();
    }
    full
}

/// 从控制台拉一个字节（**非阻塞**）：DBCN `ConsoleRead` 读 1 字节；无输入 → `None`。
///
/// 缓冲取**静态区**（原子字节防多核并发读写；栈局部量不可用——trap 栈在内核
/// 高半区）；其物理地址经**内核空间**逐段翻译（`segments`，与 [`push`] 同机制
/// ——不假设恒等映射；PULL_BUF 是内核地址，用户空间不可见，故取内核空间，
/// 其 DRAM 恒等映射涵盖 .bss）。SBI 返回值 = 实际读到字节数（0 = 无输入可用）。
static PULL_BUF: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

pub(crate) fn pull() -> Option<u8> {
    let va = VirtAddr::from_raw(&PULL_BUF as *const _ as usize);
    let (pa, _flags, _len) = crate::work::unit::team::kernel()?.space.segments(va, 1).next()?;
    let r = DbcnCall::new(Dbcn::ConsoleRead)
        .args(SArgs {
            a0: 1,
            a1: pa.as_usize(),
            ..Default::default()
        })
        .call();
    match r {
        Ok(n) if n > 0 => Some(PULL_BUF.load(core::sync::atomic::Ordering::Relaxed)),
        _ => None,
    }
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
