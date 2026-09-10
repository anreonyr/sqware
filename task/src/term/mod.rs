//! Terminal（term）— 宿主终端的 ANSI 渲壳 + 键盘输入的行编辑。
//!
//! 分层（两个方向各自成文件，输出侧不知道输入侧）：
//!   - 本文件 = **输出**：ANSI 转义生成（清屏/光标/前景色/复位）+ 裸写/写行，
//!     以及 [`Terminal`] 上 `readline` 的转发点；
//!   - [`input`] = **输入**：VTE 键盘解码（`anstyle_parse::Perform` → [`input::Key`]）
//!     + 行编辑缓冲（[`input::Line`]）+ 重绘。
//!
//! Terminal 是**唯一**与宿主 console 打交道的中间层：Shell / Lisp 等交互程序都
//! **经本模块**读写。成对关系：`read` ↔ `write`（原始字节）、`readline(prompt)` ↔
//! `writeline`（一行，`readline` 收纳 prompt 并在重绘时保留）。
//!
//! 关键：宿主终端（QEMU -nographic 所在的真实终端）**本身就是 ANSI 渲染器**，
//! 无需自绘屏幕 buffer。我们只产转义、不解析渲染。
//!
//! no_std：无 std，仅 `anstyle_parse`（default-features = false）+ alloc（format!）。

use alloc::format;

use crate::env::io::put;

pub mod input;

pub use input::Readline;

// ── 输出（ANSI 转义生成）──

/// 前景色（ANSI 三一色 30-37）。
#[derive(Clone, Copy, Debug)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

impl Color {
    fn code(self) -> u8 {
        match self {
            Color::Black => 30,
            Color::Red => 31,
            Color::Green => 32,
            Color::Yellow => 33,
            Color::Blue => 34,
            Color::Magenta => 35,
            Color::Cyan => 36,
            Color::White => 37,
        }
    }
}

/// ANSI 渲壳 + 行编辑入口。纯输出方法无状态；`readline` 内部自建解码器/行缓冲。
#[derive(Default, Clone, Copy)]
pub struct Terminal;

impl Terminal {
    /// 裸写（不加 `\n`）：供提示符等「无需换行的片段」输出。与 [`read`] 对称。
    pub fn write(&self, s: &str) {
        put(s).ok();
    }

    /// 清屏 + 光标回 home（`ESC[2J` + `ESC[H`）。
    pub fn clear(&self) {
        self.write("\x1b[2J\x1b[H");
    }

    /// 前景色（`ESC[3xm`）。
    pub fn fg(&self, color: Color) {
        self.write(&format!("\x1b[{}m", color.code()));
    }

    /// 复位 SGR（`ESC[0m`）。
    pub fn reset(&self) {
        self.write("\x1b[0m");
    }

    /// 写一行（自动追加 `\n`）。与 [`Terminal::readline`]（读一行、不含 `\n`）对称。
    pub fn writeline(&self, s: &str) {
        self.write(&format!("{s}\n"));
    }
}

impl Terminal {
    /// 读一整行（带行编辑），**收纳 prompt**——实现见 [`input`]（输入方向归它）。
    pub fn readline(&self, prompt: &str) -> Readline {
        input::readline(self, prompt)
    }
}
