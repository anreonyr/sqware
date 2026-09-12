//! 共享入口（镜像程序引导 + panic 处理）：四个 `bin/` 共用，故住在 lib 面。

use core::arch::global_asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;

use env::NOTE_MAX;
use runtime::env::room::{exit_with, exit_with_note};

global_asm!(
    ".section .text._start",
    ".globl _start",
    "_start:",
    "    call save_args", // a0/a1 = 启动参数区（Spawn 写入）——必须在任何调用前保存
    "    call tls_bootstrap",
    "    call main",
    "    call exit_trampoline", // main 返回（理论上 !，兜底）→ room::exit
    "1: j 1b",                  // ec 返回则兜底循环
);

#[unsafe(no_mangle)]
extern "C" fn tls_bootstrap() {
    unsafe { runtime::core::tls::bootstrap() }
}

#[unsafe(no_mangle)]
extern "C" fn exit_trampoline() -> ! {
    exit_with(EXIT_OK)
}

/// 自愿结束的原因码（域自己的编号空间的 0 号）。
pub const EXIT_OK: usize = 0;

/// 域 panic 的原因码——**域自己的诊断编号**，取高位段、与各 bin 的启动握手编号
/// （小整数）不重叠：读 trace 的人一眼能分出"启动没走通"与"域自己炸了"。
pub const EXIT_PANIC: usize = 0xFFFF_FF01;

/// panic 现场的**那句话**：栈上拼，**不分配**（panic 现场禁忌照旧——`format!` 会分配，
/// 分配失败就是双重 panic）。
///
/// 为什么由域自己拼、而不是让内核去解析符号：`PanicInfo` 里的消息与 `file:line:col`
/// 都是**编译期字面量**（落在只读段里），拿到它们**不需要符号表**——这正是本仓
/// "内核侧不做符号解析"的底气（`docs/driver.md` §12）。
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

/// 域内 panic：**不打印**，把处置交给内核（`docs/driver.md` §3.3.5）——但**带上那句话**。
///
/// 一个死掉的域还能打印，前提是服务、门闩、槽全都活着——**那是错的依赖方向**：域正在
/// 告诉你它算不下去了，却要它先去求一条活路。故这里一个字节都不往控制台写；写的是
/// `Reap { reason, note }`——**内核**接住那句话，在自己的出口上打出来（那时不需要
/// 控制台活着，也不需要符号表）。
///
/// 现场的三笔账因此各有出处：谁/何时/为何 = trace 的 `RoomEvent::Exit`；`哪里` =
/// 这句话里的 `file:line:col`；寄存器现场 = 内核故障路径自己留的痕。
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut note = Note::new();
    let _ = write!(note, "{}", info.message());
    if let Some(at) = info.location() {
        let _ = write!(note, " at {}:{}:{}", at.file(), at.line(), at.column());
    }
    exit_with_note(EXIT_PANIC, note.as_str())
}
