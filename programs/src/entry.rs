//! 共享入口（镜像程序的**引导** + panic 处理）：**每个程序都共用**（各程序一声
//! `extern crate programs;` 就是为它——`use` 只带符号，不算真的链上），故住在 lib 面。
//!
//! **退场那一半不在这里**（照实记）：`main` 的返回类型 `Report` / `Exit`、以及"把它送进
//! 内核"的 `finish`，三者住 `runtime::core::exit`——判据是**内核读不读它**，那边的照实记
//! 写全了。本文件因此只剩**入口那一手**：`_start` 的汇编（引导）、[`entry`]（把生成物与
//! bin 自己的 `main` 接上）、panic 处理。
//!
//! # 那一层是**宏展开**出来的（`crates/mold`）
//!
//! bin 里写 `#[entry] fn main() …`，宏展开成两件东西（**你写的那个函数一个字没动**）：
//!
//! ```ignore
//! extern "C" fn clean_ret() { programs::entry::entry(main) }
//! ```
//!
//! ——`_start` 那句 `call clean_ret` 找的就是它。**符号名不再是 `main`**，故写程序的人既不必
//! 改名，也不必签 `#[unsafe(no_mangle)]`（展开那一侧的照实记在 `crates/mold`）。
//! 退出的三笔账里，"码从哪来、话怎么带"是 `runtime::core::exit` 的词汇，"往哪送"是本文件
//! [`entry`] 走到底那一手（`runtime::core::exit::finish`）——`room::exit` 因此全仓只有两处
//! 调用点（它，与 `runtime::core::unit` 的线程收尾）。

use core::arch::global_asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;

use env::NOTE_MAX;
use runtime::core::exit::{Exit, finish};

// 前两行与从前**一字不差**（顺序是硬的：a0/a1 是寄存器里的入参，save_args 必须最先；
// TLS 要在任何 Rust 代码碰 `tls` 之前立起来）。后两行是出口的形状；`entry` 是 `-> !`，
// 故 `1: j 1b` 那行**必须留着**（编译器不知道它不返回）。
//
// **照实记（上面那两句注释曾经说过头）**：这里原先写着"a0 = 出口槽的地址、a1 = 本 bin 那个
// `main`（只递地址，不调用）"——那是更早一版 `mod __entry` 的形状。**今天 `clean_ret` 不带
// 参数**（宏生成的就是 `extern "C" fn clean_ret()`，`main` 在**展开时**就绑好了），故 a0 那
// 一格没有读者。它照旧留着不动：撤掉要重新量栈对齐这件事，而它不影响任何行为。
global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call save_args", // a0/a1 = 启动参数区（Spawn 写入）——必须在任何调用前保存
    "    call tls_bootstrap",
    "    addi sp, sp, -8", // 出口槽：`Reason` 就住这里（8 字节，栈对齐）
    "    mv   a0, sp",
    "    call clean_ret", // `#[entry]` 展开出的那一层（形状固定：a0 = 槽）
    "1: j 1b",           // main 返回则兜底循环（它理论上不返回）
);

#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { runtime::core::tls::bootstrap() }
}

/// **入口那一手**：`bare` 是 bin 自己那个 `main`（当**函数项**传进来，不在这里调用——
/// 于是 `R = !` 那一档由类型系统自己落定，生成物里没有"调用之后还写了东西"的死码）。
///
/// 生成物里的 `clean_ret` 只写一句 `entry(crate::main)`；泛型那一层的不透明性因此不泄漏给
/// 写程序的人。折码与送内核在 `runtime::core::exit`（[`finish`]）。
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
/// **它不走 `Exit`**：panic 物理上必须 `!`，装不进"返回值"那条路——它直接调出口原语
/// （带 note 那一支），与 [`entry`] 同住这个文件、同归 `room::exit` 一处。
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut note = Note::new();
    let _ = write!(note, "{}", info.message());
    if let Some(at) = info.location() {
        let _ = write!(note, " at {}:{}:{}", at.file(), at.line(), at.column());
    }
    runtime::env::room::exit(env::EXIT_PANIC, Some(note.as_str()))
}
