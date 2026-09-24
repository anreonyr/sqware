//! 共享入口（镜像程序引导 + 退出 + panic 处理）：**每个程序都共用**（各程序一声
//! `extern crate programs;` 就是为它——`use` 只带符号，不算真的链上），故住在 lib 面。
//!
//! # 退出这件事的形状（照 `std` 那套 `Termination`，改了两处名）
//!
//! `std` 里 `main` 的返回类型是**它自己的事**（`()` / `ExitCode` / `Result` / `!` 都行），
//! 编译器生成的入口去叫 `Termination::report()` 把它折成一个 **`ExitCode`**，再交给
//! `std::process::exit`。这里一模一样，只换名与那个"折出来的数"：
//!
//! | `std` | 这里 |
//! |---|---|
//! | `Termination` | [`Exit`] |
//! | `report(self) -> ExitCode` | [`Exit::report`] → [`Reason`]（`usize`，直接就是 `Reap.reason`） |
//! | lang item `#[lang_start]` + `rustc_main` | 生成物里那个 `main` + [`entry`] |
//! | `impl Termination for ()/{!}/Result<T,E>` | 同形几条，见本文件 |
//!
//! **那一格是构建脚本生成的**（`programs/build.rs` / `harness/build.rs`）：每个 `[[bin]]`
//! 得一份 `entry_<路径>.rs`，里面只有一个
//!
//! ```ignore
//! mod __entry {
//!     #[unsafe(no_mangle)]
//!     extern "C" fn main() { programs::entry::entry(crate::main) }
//! }
//! ```
//!
//! ——`_start` 那句 `call main` 找的就是它。**裹一层模块**是为了让 bin 自己那个 `main`
//! 在同名的情况下仍够得着（`crate::main` 从任何模块都指得到）：写程序的人因此既不必改名，
//! 也不必签 `#[unsafe(no_mangle)]`。退出的三笔账（`Reason` 从哪来、note 怎么带、往哪送）
//! 全在本文件——`room::exit` 因此全仓只有两处调用点（这里与 `runtime::core::unit` 的线程收尾）。

use core::arch::global_asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;

use env::{NOTE_MAX, Reason};
use runtime::env::room::exit;

// 前两行与从前**一字不差**（顺序是硬的：a0/a1 是寄存器里的入参，save_args 必须最先；
// TLS 要在任何 Rust 代码碰 `tls` 之前立起来）。后两行是新的出口形状：
//   a0 = 出口槽的地址（本域自己的栈上，`Reason` 大小）
//   a1 = 本 bin 那个 `main`（只递地址，不调用）
//   entry 是 `-> !`，故 `1: j 1b` 那行**必须留着**（编译器不知道它不返回）。
global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call save_args", // a0/a1 = 启动参数区（Spawn 写入）——必须在任何调用前保存
    "    call tls_bootstrap",
    "    addi sp, sp, -8", // 出口槽：`Reason` 就住这里（8 字节，栈对齐）
    "    mv   a0, sp",
    "    call main", // `main` 是**非泛型**的那一层（形状固定：a0 = 槽）；泛型在它后面
    "1: j 1b", // main 返回则兜底循环（它理论上不返回）
);

#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { runtime::core::tls::bootstrap() }
}

/// `main` 的返回类型只需要这一个 trait——**每个 bin 的 `main` 是它的一个实现**。
///
/// 四条实现就是 `std` 那四条再加一条：`()` = "没有失败要报"（`EXIT_OK`）、`!` = "我自己退"
/// （`exit` 那条路走不到，但类型上必须能过）、`Reason` = 只报码、`Result<T, E>` = `?` 一路带出来。
/// **`()` 不是"忘了报"**：真要报失败的程序把 `main` 写成 `Result<(), Reason>`，
/// 那里 `()` 当不了退出码，忘了报就是编译错误。
///
/// 折出来的不是裸码而是 [`Report`]：**note 也得有地方住**——探针那族的回执就是"报码 +
/// 带一句话"，而那句话常常是**栈上现拼的**（`format!`），故 [`Report`] 借它、不要求 `'static`。
/// 这正是 `report` 取 `&self` 而不是 `self` 的理由：借出的 note 活不过一个按值消耗的 `self`。
pub trait Exit {
    fn report(&self) -> Report<'_>;
}

/// note 的**线形状**：`(ptr, len)`——`0` 长度即"无话"。
///
/// **为什么不用 `Option<&str>`**：这个词要跨 [`entry`] 那个 `extern "C" fn() -> R` 的边界
/// 回来，而 `Option<&str>` 是带 niche 的枚举、没有 `repr`，编译器（正确地）不肯认它是
/// FFI-safe。拆成两个标量（与内核 `Reap { note, len }` 同一形）就干净了。
#[derive(Clone, Copy)]
#[repr(C)]
struct RawNote {
    ptr: *const u8,
    len: usize,
}

impl RawNote {
    /// 无话。
    const NONE: Self = Self {
        ptr: core::ptr::null(),
        len: 0,
    };

    const fn of(s: &str) -> Self {
        Self {
            ptr: s.as_ptr(),
            len: s.len(),
        }
    }

    /// 解回那句话（只有 [`Report::parts`] 这一个读点，故 SAFETY 条件收在那里）。
    unsafe fn get<'a>(self) -> Option<&'a str> {
        match self.len {
            0 => None,
            n => Some(unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(self.ptr, n)) }),
        }
    }
}

/// 出口点的返回类型：**原因码 + 可选的一句话**——与 [`runtime::env::room::exit`] 的入参同形。
///
/// 它就是"`main` 想对内核说的全部"：两格，与 `Reap { reason, note }` 一一对应。
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Report<'a> {
    pub reason: Reason,
    note: RawNote,
    _borrow: core::marker::PhantomData<&'a str>,
}

impl<'a> Report<'a> {
    /// 只报码。
    pub const fn new(reason: Reason) -> Self {
        Self {
            reason,
            note: RawNote::NONE,
            _borrow: core::marker::PhantomData,
        }
    }

    /// 报码 + 一句话（"哪里算不下去"）——那句话借多久都行（只要活到出口）。
    pub const fn note(reason: Reason, note: &'a str) -> Self {
        Self {
            reason,
            note: RawNote::of(note),
            _borrow: core::marker::PhantomData,
        }
    }

    /// 拆成 [`runtime::env::room::exit`] 那两格（出口点用的就是它）。
    ///
    /// SAFETY：`note` 那一对指针要么是 [`RawNote::NONE`]，要么指着 [`Report::note`] 收进来的
    /// 那个 `&'a str`——`'_` 绑在 `&self` 上，故那句话至少活到本调用返回。
    pub fn parts(&self) -> (Reason, Option<&'a str>) {
        (self.reason, unsafe { self.note.get() })
    }
}

impl<'a> Exit for Report<'a> {
    fn report(&self) -> Report<'_> {
        Report {
            reason: self.reason,
            note: self.note,
            _borrow: core::marker::PhantomData,
        }
    }
}

impl Exit for Reason {
    fn report(&self) -> Report<'_> {
        Report::new(*self)
    }
}

impl Exit for () {
    fn report(&self) -> Report<'_> {
        Report::new(env::EXIT_OK)
    }
}

impl Exit for ! {
    fn report(&self) -> Report<'_> {
        match *self {}
    }
}

impl<T: Exit, E: Exit> Exit for Result<T, E> {
    fn report(&self) -> Report<'_> {
        match self {
            Ok(t) => t.report(),
            Err(e) => e.report(),
        }
    }
}

/// 把一份 [`Report`] 送进内核——**[`entry`] 走到底就是它**（生成物里没有泛型，
/// 这一步因此与 `main` 的返回类型无关）。
///
/// 这里同时让 `reason` 在 `exit(...)` 返回（不该发生）时留在现场，读的人不至于只看到一句
/// `unreachable`。
pub fn finish(report: Report<'_>) -> ! {
    let reason = report.reason;
    let (_, note) = report.parts();
    exit(reason, note)
}

/// **入口那一手**：`bare` 是 bin 自己那个 `main`（当**函数项**传进来，不在这里调用——
/// 于是 `R = !` 那一档由类型系统自己落定，生成物里没有"调用之后还写了东西"的死码）。
///
/// 生成物里的 `main` 只写一句 `entry(crate::main)`；泛型那一层的不透明性因此不泄漏给
/// 写程序的人。
#[inline(always)]
pub fn entry<R: Exit>(bare: fn() -> R) -> ! {
    finish(bare().report())
}

/// panic 现场的**那句话**：栈上拼，**不分配**（panic 现场禁忌照旧——`format!` 会分配，
/// 分配失败就是双重 panic）。
///
/// 为什么由域自己拼、而不是让内核去解析符号：`PanicInfo` 里的消息与 `file:line:col`
/// 都是**编译期字面量**（落在只读段里），拿到它们**不需要符号表**——这正是本仓
/// "内核侧不做符号解析"的底气。
struct Note {
    buf: [u8; NOTE_MAX],
    len: usize,
}

impl Note {
    const fn new() -> Self {
        Self {
            buf: [0; NOTE_MAX],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        // `write_str` 只在字符边界上截断 ⇒ 这里必然是全法 UTF-8（兜底分支只为不 panic）。
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("<note>")
    }
}

impl fmt::Write for Note {
    /// 定长：装不下就丢尾巴——**在字符边界上截**，否则整句话会因为尾部半个字符而作废。
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut n = s.len().min(self.buf.len() - self.len);
        while n > 0 && !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// 域内 panic：**不打印**，把处置交给内核——但**带上那句话**。
///
/// 一个死掉的域还能打印，前提是服务、门闩、槽全都活着——**那是错的依赖方向**：域正在
/// 告诉你它算不下去了，却要它先去求一条活路。故这里一个字节都不往控制台写；写的是
/// `Reap { reason, note }`——**内核**接住那句话，在自己的出口上打出来（那时不需要
/// 控制台活着，也不需要符号表）。
///
/// 现场的三笔账因此各有出处：谁/何时/为何 = trace 的 `RoomEvent::Exit`；`哪里` =
/// 这句话里的 `file:line:col`；寄存器现场 = 内核故障路径自己留的痕。
///
/// **它不走 [`Exit`]**：panic 物理上必须 `!`，装不进"返回值"那条路——它直接调出口原语
/// （带 note 那一支），与 [`entry`] 同住这个文件、同归 `room::exit` 一处。
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut note = Note::new();
    let _ = write!(note, "{}", info.message());
    if let Some(at) = info.location() {
        let _ = write!(note, " at {}:{}:{}", at.file(), at.line(), at.column());
    }
    exit(env::EXIT_PANIC, Some(note.as_str()))
}
