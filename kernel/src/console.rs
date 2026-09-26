// 控制台输出 — **内核自己的**打印 sink（SBI Dbcn 块写 + 段地址解析）
//
// 命名约定：输出用 put!/putln!。
//
// Dbcn 按物理地址读取：恒等区 VA 即 PA 直通；非恒等区的内核地址经页表译成 PA。
//
// **内核不再是域的控制台通路**：`IOCall::Put`/`Get`
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
    crate::platform::machine::dram_edge().unwrap_or(0x9000_0000)
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
            // 内核空间映射（高半区 trap 栈 / 内核堆 / 镜像恒等区外物理帧）：经
            // `Space::translate` 走页表树——**会取 Space 锁（L2）**，不是"无锁 walk"。
            // 关机审计（trap 栈，任务全退 ident=Last）与 panic 现场（其他核已停）
            // 都依赖这条路径；持 L3 锁时**不可**打印（4→2 反向嵌套当场 panic）。
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

/// 内核自己的读入出口：从调试控制台读至多 `buf.len()` 字节写进 `buf`，返实际字节数。
///
/// **只认内核半区的缓冲**（与 `put` 那两条路同款：恒等区 VA 即 PA、或经页表译）。
/// 域给的缓冲一次都到不了这里——调用方（`envcall/debug.rs`）先备内核栈暂存，读进来
/// 之后再 `copy_out` 给域。理由只有一条：DBCN 按**物理地址**读写，用户 VA 在这里没有意义。
///
/// **不阻塞**：实测（OpenSBI v1.9 / QEMU virt）没数据时**立刻返 0**，不是"等到至少一个
/// 字节"。这条差别是**空转的红线**——调用方拿到 0 若立刻再问，就是在 U 态烧一颗核
/// （实测宿主 99%，见 `programs/src/user/echo.rs` 那条注）。读入方要么睡一毫秒再来，
/// 要么等中断（那是 console 域的事）。`None` = 缓冲不可直读。
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
    // SAFETY: `pa` 指向一段可写、物理连续的缓冲（恒等区直通，或 `translate_kernel`
    // 逐页校验过物理连续性）；`Dbcn::ConsoleRead` 按 PA 写回。
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

/// 内核空间翻译 `[va, va+len)`：整段须在同一物理连续块内（每页 walk 检查
/// 连续性），否则 None。内核空间构造后页表树只读，`translate` 安全。
/// 用于非恒等区、且无活任务身份可译的高半区内核地址（trap 栈 / 内核堆缓冲）。
///
/// **仅限内核半区地址**：用户半区 VA（如用户堆 0x87xxxxxx）在内核空间虽也有
/// identity 映射，但用户空间的该 VA 映射到**不同的物理帧**（独立分配）——用
/// 内核空间翻译会得到错误的 PA。用户地址一律**直接回退**（返 `None`）——旧注写的
/// "回退活任务空间路径（`push`）"里那个 `push` 已随设备面删除，今天不存在这条替域
/// 写出去的路。
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

/// **一行攒进这具栈缓冲，再一次发出去**（[`_write`] 的照实记）。
///
/// 装不下的长行**不会丢字**：攒满那一段先冲出去，此后转成直通（分段）。那种行里没有判据。
const LINE_MAX: usize = 256;

/// 攒一行的栈缓冲（不分配）。
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

    /// 把攒下的这一段**一次**写出去（一次 `Dbcn::ConsoleWrite`）。
    fn emit(&mut self) {
        if self.len == 0 {
            return;
        }
        // 只往里追加过整段 `&str`，故这一段必是合法 UTF-8；下面那句 `else` 只是保险。
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
            // 这一行装不下 ⇒ 它本来就不是"一次写得完"的：先把攒下的冲出去，其余照旧分段。
            self.emit();
            let _ = Console.write_str(s);
        }
        Ok(())
    }
}

/// put!/putln!/log logger 的共同出口。
///
/// # 照实记（为什么先把一行攒起来，再一次发出去）
///
/// 原先这里是 `Console.write_fmt(args)` 直通。而 `fmt::Write` 是**按段**调 `write_str` 的，
/// 每一段就是一次 `Dbcn::ConsoleWrite` ⇒ **一条读数行要 2~5 次 ecall**。那几次之间是空档：本核
/// 在两次 ecall 之间被抢占（定时器 / IPI），或者别的核正好挤进那一小段，**别人那一整行就插进来
/// 了**。实测现场（`trace/gate-1790248225/run2.log:523`）：
///
/// ```text
///     board: swept n=1 occupied=4system: gone principal state=Dead ousted=true heir=13→12 wait=now
/// ```
///
/// ——第一条的后半截（就那一个 `\n`）落在了第二条**后面**。补量：12 份现场 3976 行里 **1 例**
/// （约每 13 轮一次；`soak` 默认只跑 1 轮，故那是它**假红**的一个来源——`board:` 那一族的形状
/// 锚了行尾 `$`，胶起来的行两边都不匹配）。更早那批现场里同样的胶行有 6 例（`soak-*` /
/// `framework-*` / `console-*` 各一份——那批归档由**已删的** `crates/gate` 落的），
/// **全部落在同一个位置上**：最后一段正文之后、`\n` 之前。
///
/// 收法：**一行攒进 [`Line`]，再一次发**（一行一次 ecall）。
///
/// **照实记（这一收的边界）**：它关掉的是**我们自己**制造的窗口（两次 ecall 之间）。剩下的一档是
/// "两颗核同时进 M 模式写同一个 UART"——那落在 SBI 那一侧，**今天没有证据**（那 7 例与这一轮补量
/// 的 1 例，无一例落在段中间）。若日后仍见胶行，那就该在这一层加锁，而**不是**去放宽门的形状。
pub fn _write(args: fmt::Arguments) {
    let mut line = Line::new();
    let _ = fmt::write(&mut line, args);
    line.emit();
}

/// 让 `fmt::Write` 的格式化器能把整行转发到控制台。
/// 无堆分配；**恒等区路径无色无锁，非恒等区路径会取 Space 锁（L2）**——故持 L3 锁时
/// 不可打印（4→2 反向嵌套当场 panic）。panic / 关机现场可用是因为那时没有别的执行流。
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
