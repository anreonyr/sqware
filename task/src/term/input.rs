//! 输入方向：VTE 键盘解码 + 行编辑缓冲 + 重绘。
//!
//! 与 `term`（输出/ANSI）分开的理由：本方向**不需要**任何 ANSI/颜色知识
//! （只发回车、擦行、光标左移三种控制序列），而输出方向不依赖本文件的任何类型。
//! 依赖单向：`input` → `term`（用 `Terminal::write` 回显），反向不成立。
//!
//! 数据流：`env::io::try_get` 逐字节 → [`Decoder`]（`anstyle_parse::Perform` 回调）
//! → [`Key`] 事件 → [`Sink`]（[`LineSink`]）改 [`Line`] 缓冲并 [`redraw`]。
//! 终止标志（回车/Ctrl-C/Ctrl-D）走独立 `core::cell::Cell`，避开 `&mut Line` 借用冲突。

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anstyle_parse::{Params, Parser, Perform};

use super::Terminal;
use crate::env::room::sleep;
use core::time::Duration;

// ── 输入（VTE 解码 + 行编辑）──

/// readline 的结果。
#[derive(Debug)]
pub enum Readline {
    /// 一行文本（回车提交）。
    Line(String),
    /// Ctrl-D（Eof）。
    Eof,
    /// Ctrl-C（清空当前行）。
    Interrupt,
}

/// 行缓冲 + 光标。终止标志（回车/Ctrl-C/Ctrl-D）放 [`Cell`]（独立共享引用，
/// `LineSink` 写、`readline` 读——避开 `&mut Line` 的借用冲突）。
struct Line {
    buf: Vec<char>,
    pos: usize,
}

impl Line {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            pos: 0,
        }
    }

    fn insert(&mut self, c: char) {
        self.buf.insert(self.pos, c);
        self.pos += 1;
    }
    fn backspace(&mut self) {
        if self.pos > 0 {
            self.buf.remove(self.pos - 1);
            self.pos -= 1;
        }
    }
    fn delete(&mut self) {
        if self.pos < self.buf.len() {
            self.buf.remove(self.pos);
        }
    }
    fn home(&mut self) {
        self.pos = 0;
    }
    fn end(&mut self) {
        self.pos = self.buf.len();
    }
    fn left(&mut self) {
        self.pos = self.pos.saturating_sub(1);
    }
    fn right(&mut self) {
        if self.pos < self.buf.len() {
            self.pos += 1;
        }
    }
    fn text(&self) -> String {
        self.buf.iter().collect()
    }
}

/// 按键 → 行编辑 + 回显（经 Terminal 输出）。终止标志经 [`Cell`] 写，`readline` 读。
struct LineSink<'a> {
    term: &'a Terminal,
    prompt: &'a str,
    line: &'a mut Line,
    submitted: &'a core::cell::Cell<bool>,
    interrupt: &'a core::cell::Cell<bool>,
    eof: &'a core::cell::Cell<bool>,
}

impl Sink for LineSink<'_> {
    fn on_key(&mut self, key: Key) {
        match key {
            Key::Char(c) => {
                self.line.insert(c);
                redraw(self.term, self.prompt, self.line);
            }
            Key::Backspace => {
                self.line.backspace();
                redraw(self.term, self.prompt, self.line);
            }
            Key::Delete => {
                self.line.delete();
                redraw(self.term, self.prompt, self.line);
            }
            Key::Left => {
                self.line.left();
                redraw(self.term, self.prompt, self.line);
            }
            Key::Right => {
                self.line.right();
                redraw(self.term, self.prompt, self.line);
            }
            Key::Home => {
                self.line.home();
                redraw(self.term, self.prompt, self.line);
            }
            Key::End => {
                self.line.end();
                redraw(self.term, self.prompt, self.line);
            }
            Key::Enter => {
                self.submitted.set(true);
                self.term.write("\r\n");
            }
            Key::Interrupt => {
                self.interrupt.set(true);
            }
            Key::Eof => {
                self.eof.set(true);
            }
            Key::Tab => {
                self.line.insert('\t');
                redraw(self.term, self.prompt, self.line);
            }
            // 方向键上/下对单行缓冲区无操作（v1 无历史）。
            Key::Up | Key::Down => {}
        }
    }
}

/// 重绘当前行：光标回行首 → 清到行尾 → 打印 prompt + 缓冲 → 光标定位到 pos。
fn redraw(term: &Terminal, prompt: &str, line: &Line) {
    let s: String = line.buf.iter().collect();
    term.write("\r\x1b[K");
    term.write(prompt);
    term.write(&s);
    let back = (line.buf.len() - line.pos) as u16;
    if back > 0 {
        term.write(&format!("\x1b[{back}D"));
    }
}

// ── VTE 解码 ──

/// 键盘事件（Terminal 内部，行编辑用它）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Char(char),
    Backspace,
    Delete,
    Enter,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Tab,
    Interrupt,
    Eof,
}

/// 键盘事件消费者。
trait Sink {
    fn on_key(&mut self, key: Key);
}

/// VTE 解码器：持 `anstyle_parse::Parser`，逐字节 `advance`，把解析出的
/// 转义/控制序列经 `Perform` 回调翻译成 [`Key`] 事件，转发给 `Sink`。
struct Decoder<S> {
    // default-features = false → DefaultCharAccumulator = AsciiParser（7-bit）。
    parser: Parser<anstyle_parse::AsciiParser>,
    sink: S,
}

impl<S: Sink> Decoder<S> {
    fn new(sink: S) -> Self {
        Self {
            parser: Parser::<anstyle_parse::AsciiParser>::new(),
            sink,
        }
    }

    fn advance(&mut self, byte: u8) {
        let mut p = core::mem::take(&mut self.parser);
        p.advance(self, byte);
        self.parser = p;
    }
}

impl<S: Sink> Perform for Decoder<S> {
    fn print(&mut self, c: char) {
        // AsciiParser 把 `0x7f`(DEL) 归入可打印区间 → `print`。拦截为退格，
        // 使退格键（0x7f 或 0x08）统一走 Backspace，而非插入 `\x7f` 字符。
        if c == '\x7f' {
            self.sink.on_key(Key::Backspace);
        } else {
            self.sink.on_key(Key::Char(c));
        }
    }

    fn execute(&mut self, byte: u8) {
        let key = match byte {
            b'\r' | b'\n' => Key::Enter,
            0x7f | 0x08 => Key::Backspace,
            b'\t' => Key::Tab,
            0x03 => Key::Interrupt,
            0x04 => Key::Eof,
            _ => return,
        };
        self.sink.on_key(key);
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: u8) {
        // VTE CSI 键盘序列：`ESC [ <params> <final>`。
        // 方向键 final = A/B/C/D；Home/End = H/F。
        let key = match action {
            b'A' => Key::Up,
            b'B' => Key::Down,
            b'C' => Key::Right,
            b'D' => Key::Left,
            b'H' => Key::Home,
            b'F' => Key::End,
            b'~' => {
                // `ESC [ n ~`：Home(1)/Insert(2)/Delete(3)/End(4)/PgUp(5)/PgDn(6)。
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first().copied())
                    .unwrap_or(0);
                match n {
                    1 => Key::Home,
                    4 => Key::End,
                    3 => Key::Delete,
                    _ => return,
                }
            }
            _ => return,
        };
        self.sink.on_key(key);
    }

    // OSC/DCS 无操作（Terminal 不处理系统命令）。
    fn hook(&mut self, _p: &Params, _i: &[u8], _ig: bool, _a: u8) {}
    fn put(&mut self, _b: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _p: &[&[u8]], _b: bool) {}
    fn esc_dispatch(&mut self, _i: &[u8], _ig: bool, _b: u8) {}
}

/// 读一整行（带行编辑），**收纳 prompt**。
///
/// 先打 `prompt`，行编辑过程中每次重绘都用 `\r\x1b[K + prompt + 输入串`
/// （清行/退格/光标移动时 prompt 不丢）。阻塞直到：
/// - 回车提交 → [`Readline::Line`]；
/// - Ctrl-C 清行 → [`Readline::Interrupt`]；
/// - Ctrl-D 退出 → [`Readline::Eof`]。
pub fn readline(term: &Terminal, prompt: &str) -> Readline {
    term.write(prompt);
    let mut ed = Line::new();
    let submitted = core::cell::Cell::new(false);
    let interrupt = core::cell::Cell::new(false);
    let eof = core::cell::Cell::new(false);
    {
        let mut dec = Decoder::new(LineSink {
            term,
            prompt,
            line: &mut ed,
            submitted: &submitted,
            interrupt: &interrupt,
            eof: &eof,
        });
        loop {
            if let Some(b) = crate::env::io::try_get() {
                dec.advance(b);
            } else {
                let _ = sleep(Duration::from_millis(1));
            }
            if submitted.get() {
                break;
            }
            if interrupt.get() {
                term.write("\r\n");
                break;
            }
            if eof.get() {
                break;
            }
        }
    } // dec 在此 drop，释放对 `ed` 的 &mut 借用
    if interrupt.get() {
        return Readline::Interrupt;
    }
    if eof.get() {
        return Readline::Eof;
    }
    Readline::Line(ed.text())
}
