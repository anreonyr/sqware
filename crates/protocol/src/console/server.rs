//! console·server — 控制台服务：ANSI 渲壳 + VTE 键盘解码 + 行编辑 + 客户端表。
//!
//! # 为什么要两线程
//!
//! 第一版把 `ReadLine` 写成**就地阻塞**（单线程），于是**等输入期间服务不理别的请求**：
//! 任何 `Write` 都排在请求孔里 ⇒ **消息不显示**，屏幕只剩反复重绘的提示符。
//! 而"别的程序能打印"正是控制台存在的理由，所以这不是取舍、是缺陷。
//!
//! 现在分成两半，各自只持锁一小段：
//!
//! ```text
//! 请求线程   pull 请求孔 → Open/Write/Close/ReadLine → 回信孔
//! 输入线程   读 UART → 解码 → 改行缓冲 → 重绘 → 回车时把整行回给等读的会话
//! ```
//!
//! 两者共享 [`State`]（`Lock`）。**输入线程每轮只持锁一拍**（poll 一次设备、
//! 处理一个字节），故输出请求随时能插进来。
//!
//! # 输出撞上输入时重绘
//!
//! 有会话在等读时，`Write` 先 `\r\x1b[K` 清掉正在编辑的那一行、打印消息、
//! 再把 `prompt + 缓冲` 重画回去（[`State::write`]）。这就是"别的程序打印时
//! 不弄丢你正在打的字"。
//!
//! # 两枚孔的契约
//!
//! ```text
//! 请求孔   客户端 push 请求   → 服务 pull
//! 回信孔   服务 push 回复     → 客户端 pull    （Open 时登记它的对端 token）
//! ```
//!
//! 回信孔的 token **只在请求线程的表里**——故整行由输入线程放进共享态、请求线程来推
//! （踩过）。**跨 task 交接句柄这条路走不通**，别再试第三次。

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anstyle_parse::{Params, Parser, Perform};

use runtime::env::io;

use super::wire::{LINE_MAX, PAYLOAD_LEN, Reply, Request};

/// 同时在线的客户端上界（会话 id = 槽位 + 1）。
const MAX_CLIENTS: usize = 8;

/// 输入线程的轮询周期（也是它持锁的粒度）。
pub const TICK_MS: usize = 1;

/// 一行缓冲的上界（防无界增长）。超界插入无操作。
const LINE_CAP: usize = 512;

// ── 输出（ANSI 渲壳）──

/// ANSI 渲壳：把字符串写进 UART。**服务侧唯一写设备的出口**。
///
/// 旧 `term::Terminal` 还有 `clear`/`fg`/`reset`（清屏与前景色）——搬来时查了消费者：
/// **零调用点**，按本仓裁决删（"真死的删、该留的写清理由"）；要用时一条转义序列就能加回来。
struct Render;

impl Render {
    fn write(&self, s: &str) {
        io::put(s).ok();
    }
}

// ── 行编辑状态（共享）──

/// 一个已开会话：**回信孔**在服务侧的 token（服务往它推回复与整行）。
///
/// 存 token 值而非 `HolePie` 句柄：`HolePie` 没有 `Drop`（`from_token` 是零成本重建），
/// 故"即建即弃"与持有等价；而 token 是 `Copy`，槽表因此可复制、不与借用打架。
#[derive(Clone, Copy)]
struct Slot {
    reply: usize,
}

/// 行缓冲 + 光标（正在编辑的那一行）。
#[derive(Default)]
struct Line {
    buf: Vec<char>,
    pos: usize,
}

impl Line {
    fn insert(&mut self, c: char) {
        if self.buf.len() >= LINE_CAP {
            return;
        }
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

/// 键盘事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
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

/// 一条回复 + 它该走**哪个会话的回信孔**。
///
/// 控制台协议**没有**请求孔回信通道：回复一律走调用方在 `Open` 时登记的那枚回信孔。
/// `to_client: None` 只出现在"还没有会话可回"的场合（`Open` 失败、消息非法）——
/// 那时回复无处可去，丢弃，客户端等到超时。
pub struct Outcome {
    /// `None` = 这一条**不回复**：`ReadLine` 已登记等读，整行由输入线程交付。
    pub reply: Option<Reply>,
    pub to_client: Option<usize>,
}

impl Outcome {
    fn quiet() -> Self {
        Self {
            reply: None,
            to_client: None,
        }
    }
    fn deny() -> Self {
        Self {
            reply: Some(Reply::Denied),
            to_client: None,
        }
    }
}

/// 等读会话的行状态。
struct Reading {
    client: usize,
    prompt: String,
    line: Line,
}

/// 服务共享状态：客户端表 + 正在编辑的那一行。**两个线程共享它**（各持一把 `Lock`）。
pub struct State {
    slots: [Option<Slot>; MAX_CLIENTS],
    reading: Option<Reading>,
    /// 输入线程完成一整行后**放在这里**，等请求线程来取（见 [`State::set_pending`]）。
    pending: Option<(usize, Reply)>,
    term: Render,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// `const`：服务把它放进 `static`（`Lock<State>`）让两个线程共享。
    pub const fn new() -> Self {
        Self {
            slots: [None; MAX_CLIENTS],
            reading: None,
            pending: None,
            term: Render,
        }
    }

    /// 输入线程用：登记"这一行该回给谁、回什么"。
    ///
    /// **不在这里推回信孔**：回信孔的 token 在**请求线程**的 pie 表里，输入线程推
    /// 不动（踩过）。整行经共享内存交给请求线程，由它推。
    pub fn set_pending(&mut self, client: usize, reply: Reply) {
        self.pending = Some((client, reply));
    }

    /// 请求线程用：取走待交付的整行（没有则 `None`）。
    pub fn take_pending(&mut self) -> Option<(usize, Reply)> {
        self.pending.take()
    }

    /// 服务自己的打印（**设备持有者用裸写**，不经协议）。
    pub fn banner(&self, s: &str) {
        self.term.write(s);
    }

    /// 处理一条请求。`ReadLine` 只登记等读、**不阻塞**（阻塞会让别的消息排队）。
    pub fn serve(&mut self, msg: &[u8]) -> Outcome {
        match Request::decode(msg) {
            Ok(request) => self.handle(request),
            Err(_) => Outcome::deny(),
        }
    }

    fn index(&self, client: usize) -> Option<usize> {
        if client == 0 || client > MAX_CLIENTS {
            return None;
        }
        self.slots[client - 1].map(|_| client - 1)
    }

    fn handle(&mut self, request: Request) -> Outcome {
        match request {
            Request::Open { reply } => self.open(reply),
            Request::Write {
                client,
                len,
                payload,
            } => self.write(client, len, &payload),
            Request::ReadLine {
                client,
                len,
                prompt,
            } => self.readline(client, len, &prompt),
            Request::Close { client } => self.close(client),
        }
    }

    fn open(&mut self, reply: usize) -> Outcome {
        if reply == 0 {
            return Outcome::deny();
        }
        match self.slots.iter().position(Option::is_none) {
            Some(i) => {
                self.slots[i] = Some(Slot { reply });
                // **去向必须是新建会话的回信孔**：回执只有经它才到得了客户端。
                // （第一版写成 `None` = 丢弃 ⇒ 客户端每条请求都等到超时；实测过。）
                Outcome {
                    reply: Some(Reply::Ok { client: i + 1 }),
                    to_client: Some(i + 1),
                }
            }
            // 表满：请求合法、服务无容量。协议里没有"稍后重试"的码，故借
            // `NoSuchClient`——客户端看到的仍是"没开成"。
            None => Outcome {
                reply: Some(Reply::NoSuchClient),
                to_client: None,
            },
        }
    }

    fn close(&mut self, client: usize) -> Outcome {
        let Some(i) = self.index(client) else {
            return Outcome {
                reply: Some(Reply::NoSuchClient),
                to_client: None,
            };
        };
        self.slots[i] = None;
        if self.reading.as_ref().is_some_and(|r| r.client == client) {
            self.reading = None;
        }
        Outcome {
            reply: Some(Reply::Ok { client }),
            to_client: Some(client),
        }
    }

    /// 写设备：**本服务是唯一写者**。有会话在等读时**先擦当前行、打印、再重画它**。
    ///
    /// 回执即**同步点**：客户端收到 Ok 才知道这段已落屏。
    fn write(&mut self, client: usize, len: usize, payload: &[u8; PAYLOAD_LEN]) -> Outcome {
        if self.index(client).is_none() {
            // 回执走**该会话的回信孔**——它正是"这个 id 不认识"的原因，故无处可推。
            // 这一支只在客户端用错 id 时出现（排查中真实撞到过）。
            return Outcome {
                reply: Some(Reply::NoSuchClient),
                to_client: None,
            };
        }
        let n = if len > PAYLOAD_LEN { PAYLOAD_LEN } else { len };
        match core::str::from_utf8(&payload[..n]) {
            Ok(s) => {
                // 正在编辑 → 先清行（否则消息会插进用户正在打的输入串中间）
                if self.reading.is_some() {
                    self.term.write("\r\x1b[K");
                }
                self.term.write(s);
                self.redraw();
                Outcome {
                    reply: Some(Reply::Ok { client }),
                    to_client: Some(client),
                }
            }
            // 非 UTF-8：逐字节写会把转义串打成碎片，故按"不可渲染"拒收。
            // 旧的 `io::put` 收 `&str`，调用方本来就给不出这种载荷。
            Err(_) => Outcome {
                reply: Some(Reply::Denied),
                to_client: Some(client),
            },
        }
    }

    /// 读一行：**只登记**。整行由输入线程在回车/Ctrl-C/Ctrl-D 时交付。
    ///
    /// 提示符**由客户端先同步写过一次**（那条 `Write` 的 Ok 即"已落屏"）；
    /// 这里只把它登记下来供重绘使用——重绘是"清行后整行重写"，故不会重复显示。
    fn readline(&mut self, client: usize, len: usize, prompt: &[u8; PAYLOAD_LEN]) -> Outcome {
        if self.index(client).is_none() {
            return Outcome {
                reply: Some(Reply::NoSuchClient),
                to_client: Some(client),
            };
        }
        if self.reading.is_some() {
            // 行编辑是**单读者**语义：已有会话在等读时不排队。
            return Outcome {
                reply: Some(Reply::NoSuchClient),
                to_client: Some(client),
            };
        }
        let n = if len > PAYLOAD_LEN { PAYLOAD_LEN } else { len };
        let prompt = core::str::from_utf8(&prompt[..n]).unwrap_or("");
        self.reading = Some(Reading {
            client,
            prompt: String::from(prompt),
            line: Line::default(),
        });
        Outcome::quiet()
    }

    /// 重画当前输入行（`\r\x1b[K` + prompt + 缓冲 + 光标定位）。
    fn redraw(&self) {
        let Some(r) = self.reading.as_ref() else {
            return;
        };
        let s = r.line.text();
        self.term.write("\r\x1b[K");
        self.term.write(&r.prompt);
        self.term.write(&s);
        let back = (r.line.buf.len() - r.line.pos) as u16;
        if back > 0 {
            self.term.write(&format!("\x1b[{back}D"));
        }
    }

    /// 有没有会话在等读（输入线程据此决定读不读设备）。
    pub fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// 处理一个按键。回车/Ctrl-C/Ctrl-D → 返回该会话的整行结果（供输入线程交付）。
    pub fn on_key(&mut self, key: Key) -> Option<(usize, Reply)> {
        let terminal = matches!(key, Key::Enter | Key::Interrupt | Key::Eof);
        {
            let r = self.reading.as_mut()?;
            match key {
                Key::Char(c) => {
                    r.line.insert(c);
                    self.redraw();
                }
                Key::Backspace => {
                    r.line.backspace();
                    self.redraw();
                }
                Key::Delete => {
                    r.line.delete();
                    self.redraw();
                }
                Key::Left => {
                    r.line.left();
                    self.redraw();
                }
                Key::Right => {
                    r.line.right();
                    self.redraw();
                }
                Key::Home => {
                    r.line.home();
                    self.redraw();
                }
                Key::End => {
                    r.line.end();
                    self.redraw();
                }
                Key::Enter => {
                    self.term.write("\r\n");
                }
                // Ctrl-C / Ctrl-D **也要换行**：这两个键结束的是一整行，若不换行，
                // 客户端下一轮的重绘是 `\r\x1b[K`（**回到本行行首**）⇒ 新提示符把
                // 上一个提示符原地盖掉，用户看到"Ctrl-C 之后没有新行"。
                // 回车那一支同理——三个收尾键在这一件事上必须一致。
                Key::Interrupt | Key::Eof => {
                    self.term.write("\r\n");
                }
                Key::Tab => {
                    r.line.insert('\t');
                    self.redraw();
                }
                // 方向键上/下对单行缓冲区无操作（v1 无历史）。
                Key::Up | Key::Down => {}
            }
        }
        if !terminal {
            return None;
        }
        let r = self.reading.take()?;
        let reply = match key {
            Key::Interrupt => Reply::Interrupt,
            Key::Eof => Reply::Eof,
            _ => {
                let bytes = r.line.text().into_bytes();
                let n = if bytes.len() > LINE_MAX {
                    LINE_MAX
                } else {
                    bytes.len()
                };
                let mut payload = [0u8; LINE_MAX];
                payload[..n].copy_from_slice(&bytes[..n]);
                Reply::Line { len: n, payload }
            }
        };
        Some((r.client, reply))
    }

    /// 该会话的回信孔 token（输入线程交付整行用）。
    pub fn reply_token(&self, client: usize) -> Option<usize> {
        let i = self.index(client)?;
        self.slots[i].map(|s| s.reply)
    }
}

/// 输入线程用：VTE 解码器。
///
/// **必须住在输入线程**——`Parser` 不是 `Send`，进不了共享态；也因此输入线程
/// 自己在栈上持有它，跨多次读行复用（解码状态在序列中途被打断时才有意义）。
pub struct Decoder {
    parser: Parser<anstyle_parse::AsciiParser>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            parser: Parser::<anstyle_parse::AsciiParser>::new(),
        }
    }

    /// 喂一个字节，产出它翻成的按键（可能没有）。
    pub fn advance(&mut self, byte: u8) -> Option<Key> {
        let mut sink = OneKey { key: None };
        let mut p = core::mem::take(&mut self.parser);
        p.advance(&mut sink, byte);
        self.parser = p;
        sink.key
    }
}

/// 单键收集器：一次 `advance` 至多产出一个键。
struct OneKey {
    key: Option<Key>,
}

impl Perform for OneKey {
    fn print(&mut self, c: char) {
        // AsciiParser 把 `0x7f`(DEL) 归入可打印区间 → `print`。拦截为退格，
        // 使退格键（0x7f 或 0x08）统一走 Backspace，而非插入 `\x7f` 字符。
        self.key = Some(if c == '\x7f' {
            Key::Backspace
        } else {
            Key::Char(c)
        });
    }

    fn execute(&mut self, byte: u8) {
        self.key = match byte {
            b'\r' | b'\n' => Some(Key::Enter),
            0x7f | 0x08 => Some(Key::Backspace),
            b'\t' => Some(Key::Tab),
            0x03 => Some(Key::Interrupt),
            0x04 => Some(Key::Eof),
            _ => None,
        };
    }

    fn csi_dispatch(&mut self, params: &Params, _i: &[u8], _ig: bool, action: u8) {
        // VTE CSI 键盘序列：`ESC [ <params> <final>`。
        // 方向键 final = A/B/C/D；Home/End = H/F。
        self.key = match action {
            b'A' => Some(Key::Up),
            b'B' => Some(Key::Down),
            b'C' => Some(Key::Right),
            b'D' => Some(Key::Left),
            b'H' => Some(Key::Home),
            b'F' => Some(Key::End),
            b'~' => {
                // `ESC [ n ~`：Home(1)/Insert(2)/Delete(3)/End(4)/PgUp(5)/PgDn(6)。
                let n = params
                    .iter()
                    .next()
                    .and_then(|p| p.first().copied())
                    .unwrap_or(0);
                match n {
                    1 => Some(Key::Home),
                    4 => Some(Key::End),
                    3 => Some(Key::Delete),
                    _ => None,
                }
            }
            _ => None,
        };
    }

    // OSC/DCS 无操作（控制台不处理系统命令）。
    fn hook(&mut self, _p: &Params, _i: &[u8], _ig: bool, _a: u8) {}
    fn put(&mut self, _b: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _p: &[&[u8]], _b: bool) {}
    fn esc_dispatch(&mut self, _i: &[u8], _ig: bool, _b: u8) {}
}
