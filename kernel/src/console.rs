// 控制台输出 — **内核自己的**打印 sink（SBI Dbcn 块写 + 段地址解析）
//
// 命名约定：输出用 put!/putln!。
//
// Dbcn 按物理地址读取：恒等区 VA 即 PA 直通；非恒等区的内核地址经页表译成 PA。
//
// **内核不再是域的控制台通路**（`docs/driver.md` §10 第三步）：`IOCall::Put`/`Get`
// 与 `push`/`pull` 一起删了——设备的持有者是 console 服务，客户端走它自己的协议。
// 本文件只剩内核**自己**的打印（banner、故障、审计），那是内核的诊断面，走固件的
// SBI，与域持有 UART 这件事互不干扰。
use core::fmt::{self, Write};

use sbi::{DbcnCall, ecall::SArgs, fid::Dbcn};

use crate::memory::manager::addr::VirtAddr;

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
        }
        // 落到这里 = 缓冲既不在恒等区、也不在内核半区（多半是格式化时引用了用户
        // 内存里的字符串）。**静默丢弃**：内核打印不该依赖域的空间是否还在，而
        // "替域把它写出去"那条路（旧 `push`，经 SBI 逐段翻译用户页）随设备面一起
        // 删了——现在往设备写是持设备者的事。
        Ok(())
    }
}

/// 内核空间翻译 `[va, va+len)`：整段须在同一物理连续块内（每页 walk 检查
/// 连续性），否则 None。内核空间构造后页表树只读，`translate` 安全。
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
    let (pa0, _) = space.translate(VirtAddr::from_raw(va))?;
    // 逐页校验物理连续性：物理帧可能不连续，须整段连续方可一次 Dbcn 直读。
    let mut va_cur = va;
    let end = va + len;
    while va_cur < end {
        let (pa, _) = space.translate(VirtAddr::from_raw(va_cur))?;
        if pa.as_usize() != pa0.as_usize() + (va_cur - va) {
            return None; // 物理不连续：退回静默（不逐段拼）
        }
        // 跳到下一页（跨过本页剩余部分）
        va_cur = (va_cur & !(crate::memory::PAGE_SIZE - 1)) + crate::memory::PAGE_SIZE;
    }
    Some(pa0.as_usize())
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
