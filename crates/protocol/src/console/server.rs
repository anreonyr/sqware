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
//! 请求线程   pull 请求孔 → Open/Write/Close/ReadLine → 出帧落屏 → 回信孔
//! 输入线程   收驱动的投递孔 → 解码 → 改行缓冲 → 重绘（只进出帧槽）
//!            → 回车时把整行放进共享态
//! ```
//!
//! 两者共享 [`State`]（`Lock`）。**输入线程每轮只持锁一拍**（收一拍字节、处理一个
//! 字节），故输出请求随时能插进来。
//!
//! 落屏只归**请求线程**：写给设备是一次跨域协议调用，要持 per-task 的门闩，而输入
//! 线程手里没有那枚门闩——与回信孔同一条规矩（"跨 task 交接句柄这条路走不通"）。
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

use env::{PieToken, TaskId};
use runtime::env::mail::{AnyPie as _, HolePie};

use super::open;
use super::wire::{ADDRESS_AT, Query, Reply, Text, WORD};
use crate::session::Session;

/// 握手帧地址槽里那一格 `usize`（LE）。解码已保证帧长，故越界这一支不可达。
fn word(m: &[u8], at: usize) -> usize {
    usize::from_le_bytes(m[at..at + WORD].try_into().unwrap_or([0u8; WORD]))
}

/// 同时在线的客户端上界（会话 id = 槽位 + 1）。
const MAX_CLIENTS: usize = 8;

/// 输入线程的轮询周期（也是它持锁的粒度）。
pub const TICK_MS: usize = 1;

/// 一行缓冲的上界（防无界增长）。超界插入无操作。
const LINE_CAP: usize = 512;

// ── 输出（ANSI 渲壳）──
//
// 渲壳只在字符串上做转义，字节落到 `State::out`（出帧槽）。见该字段的注。

// ── 行编辑状态（共享）──

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

/// 一条回复 + 它该走**哪一枚回信孔**。
///
/// 控制台协议**没有**请求孔回信通道：回复一律走该会话在 `Open` 时登记的那枚回信孔。
///
/// `to_reply` **只有一种读法：这条会话**（它的回复孔在本线程表里）。这一条是**类型纪律**，不是
/// 约定，两处旧病都归它：
///
/// - 早先这个字段装的是**会话号**，而拒绝路径往里塞的是**推者 task id**（两种命名空间挤同
///   一个 `Option<usize>`），而 `route` 只按会话号读 ⇒ 常见情况下那句话根本送不出去（task id
///   大多大于 `MAX_CLIENTS`），罕见情况下推进**别人的**回信孔——"回复孔里多出一帧"正是让
///   下一次往返错位的那类事故。
/// - 装会话号还要求**投递时再查一次表**，于是`close` 清完槽再回话就找不到孔了：那句 `Ok`
///   被丢掉，客户端白等满上界（`session` 自检第一次跑就现形）。现在**判定那一刻**就把孔定
///   下来，迟到的表变化影响不到已经做出的决定。
///
/// 凡是"问不出会话"的场合，一律由 [`State::session_of`] 从**推者**求出会话、再求它的孔
/// （`None` = 它根本没有会话，无处可回）。
pub struct Outcome {
    /// `None` = 这一条**不回复**：`ReadLine` 已登记等读，整行由输入线程交付。
    pub reply: Option<Reply>,
    pub to_reply: Option<Session>,
    /// 推完这条回执就把这条会话收场（`Close` 那一支：它已经不在表里了，只剩这枚孔要放）。
    pub closing: bool,
}

impl Outcome {
    fn quiet() -> Self {
        Self {
            reply: None,
            to_reply: None,
            closing: false,
        }
    }

    /// 回一条，且这条会话**还活着**（推完不收场）。
    fn say(reply: Reply, to_reply: Option<Session>) -> Self {
        Self {
            reply: Some(reply),
            to_reply,
            closing: false,
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
    slots: [Option<Session>; MAX_CLIENTS],
    reading: Option<Reading>,
    /// 输入线程完成一整行后**放在这里**，等请求线程来取（见 [`State::set_pending`]）。
    pending: Option<(usize, Reply)>,
    /// **出帧槽**：渲染出来的字节先攒在这里，由请求线程在推回复之前统一送出去
    /// （[`State::take_out`]）。
    ///
    /// 为什么不让渲染直接落设备：设备在一个**自己的域**里（`prog-uart`），写它是一次
    /// 跨域协议调用，而调用要持 per-task 的门闩——**只有请求线程推得动**（与回信孔
    /// 同一条规矩，踩过）。两个线程都会渲染（请求线程渲染消息、输入线程渲染行编辑），
    /// 故渲染一律只动这块共享缓冲，落屏只归请求线程。
    out: Vec<u8>,
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
            out: Vec::new(),
        }
    }

    /// 渲壳的唯一出口：把一段字符串追加进出帧槽。
    fn emit(&mut self, s: &str) {
        self.out.extend_from_slice(s.as_bytes());
    }

    /// 请求线程用：取走这一轮攒下的出帧（没有则空 `Vec`）。
    ///
    /// **必须在推回复之前调**：客户端 `Write` 的 `Ok` 含义是"这段已落屏"，
    /// 出帧没送出去就回执，等于把那条承诺改成"排队成功"（§4 否 TX 环的同一条理由）。
    pub fn take_out(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
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

    /// 服务自己的打印（`prog-console` 上线时说一句；同样只进出帧槽）。
    pub fn banner(&mut self, s: &str) {
        self.emit(s);
    }

    /// 处理一条请求。`ReadLine` 只登记等读、**不阻塞**（阻塞会让别的消息排队）。
    ///
    /// `from` = 内核在 `Pull` 时盖章的推者（task id）：`Open` **按它**开回信孔并记下归属，
    /// 其余动词**按它**核"这条会话是不是你的"（见 [`State::handle`]）。
    pub fn serve(&mut self, from: usize, msg: &[u8]) -> Outcome {
        match Query::decode(msg) {
            // 认领号与**握手孔**都在 `Open` 那一帧里（`CLIENT_AT` 与地址槽）。
            Ok(Query::Open { nonce }) => self.open(from, word(msg, ADDRESS_AT), nonce),
            Ok(query) => self.handle(from, query, msg),
            Err(_) => self.refuse(from),
        }
    }

    /// 会话号 → 槽位下标（**只判在不在**，归属另算——见 [`State::handle`]）。
    fn index(&self, client: usize) -> Option<usize> {
        if client == 0 || client > MAX_CLIENTS {
            return None;
        }
        self.slots[client - 1].map(|_| client - 1)
    }

    /// **推者 → 它那条会话的号**：会话的归属是内核盖的章，不是报文里那个数。
    ///
    /// 一个推者至多一条会话（见 [`State::open`]），故这里返的是唯一解。
    fn session_of(&self, from: usize) -> Option<usize> {
        if from == 0 {
            return None;
        }
        self.slots
            .iter()
            .position(|s| s.is_some_and(|s| s.peer().get() == from))
            .map(|i| i + 1)
    }

    /// 拒一条请求：**回执只能走推者自己那条会话**。
    ///
    /// 问不出会话的场合（号不认识 / 为 0 / 报文不合 / 不是它的会话）只有这一条路能把那句
    /// 拒绝送回去；推者根本没有会话时就无处可回（`to_reply: None`），客户端等到上界。
    fn refuse(&self, from: usize) -> Outcome {
        Outcome::say(
            Reply::Denied,
            self.session_of(from).and_then(|c| self.session(c)),
        )
    }

    /// 正文那一段：`Query::decode` 只认出"哪个动词、哪个会话"，字节本身按同一个偏移
    /// 就地取——**长度由帧长给**，故这里没有第二把尺子。
    ///
    /// **归属闸**在这一处，盖住三个动词：会话号是服务端发的，客户端只回述；而回述可以被
    /// 伪造（同一域里另一个任务猜一个号），故认的仍是**推者**——`Session::peer`。不合的请求
    /// 一律 [`State::refuse`]（那句话走推者自己那条会话，不走它冒充的那条）。
    fn handle(&mut self, from: usize, query: Query, msg: &[u8]) -> Outcome {
        let owner_ok = |s: &Self, client: usize| {
            s.index(client)
                .is_some_and(|i| s.slots[i].is_some_and(|sess| sess.peer().get() == from))
        };
        match query {
            // `Open` 由 [`State::serve`] 直接分派（它要多一个参数），到不了这里。
            Query::Open { .. } => self.refuse(from),
            Query::Write { client, .. } if owner_ok(self, client) => {
                self.write(client, Query::text(msg))
            }
            Query::ReadLine { client, .. } if owner_ok(self, client) => {
                self.readline(client, Query::text(msg))
            }
            Query::Close { client } if owner_ok(self, client) => self.close(client),
            _ => self.refuse(from),
        }
    }

    /// 开一条会话。`addr` = 请求帧地址槽里那个数，**只用来判"要不要现开回信孔"**。
    ///
    /// # 回信孔由服务端开（本协议这一版的判据）
    ///
    /// 回信孔服务于"服务端出话、客户端收话"，故它的开者必须是**服务端**：
    ///
    /// - 服务端是开者 ⇒ 服务退场时内核的寿命边封印它（"开者退场 ⇒ 它开的资源一起
    ///   封印"）⇒ 睡在它上面的客户端当场拿到 `Dead`，会话可被识别为"断了"并重连；
    /// - 反过来（客户端开）时，服务被打死这扇门**不死**，客户端永久挂在 `Busy` 上
    ///   ——「对端还没回」与「对端已经没了」不可区分。实测现象就是 `kill console`
    ///   之后 shell 再也不返回。
    ///
    /// 交接手法见 [`open::grant`]：新孔在**双方**表里各有一枚，
    /// 服务端留源、客户端那枚的句柄经入口孔交回。
    ///
    /// # 一个推者至多一条会话：在座就**就地换新**
    ///
    /// 会话的归属是推者，故同一个推者再开一次不是"第二条会话"，是**这条会话换了一条**
    /// ——旧回复孔随 `close` 回收、槽复用。这一条是量出来的：客户端每重连一次都要开一条会话，
    /// 若每次新占一格，`MAX_CLIENTS` 就被"重连次数"吃穿（同一条实例上第五次开就没了），
    /// 而表按**客户端数**有界才是对的。
    fn open(&mut self, from: usize, control: usize, nonce: u64) -> Outcome {
        if from == 0 {
            return Outcome::quiet();
        }
        // `control` = 对端在握手帧**地址槽**里递过来的那枚孔（本表里的号）：那条回执走它。
        // **不走请求孔**——请求孔是本服务的收件箱，请求循环正在上面 `pull`；推进去等于
        // 推给自己，实测会被自己吸回去（见 `open::open` 的注）。0 = 对端没给，无处可回。
        if control == 0 {
            return Outcome::quiet();
        }
        let slot = self
            .session_of(from)
            .map(|client| client - 1)
            .or_else(|| self.slots.iter().position(Option::is_none));
        // 表满：请求合法、服务无容量。**先判容量再开孔**——反过来会白开一枚孔（那枚号的
        // 句柄已经交回客户端，而回执无处可推），客户端空等满上界。协议里没有"稍后重试"
        // 的码，故借 `NoSuchClient`：客户端看到的仍是"没开成"。
        let Some(i) = slot else {
            return Outcome::say(Reply::NoSuchClient, None);
        };
        // 存进 `Session` 的**必须是本服务自己那枚**（自己表里的号）：`route` 拿它在本线程的
        // 表里找孔来推。对端那枚的号（`To::seed`）是**另一张表**里的号——存错的表现是每次
        // 推回复都被判 `Denied` 并静默丢掉，客户端只看到"没开成"（实测：`[c1] open err
        // code=-1`，而服务端每一条 `Open` 都成功开了会话）。
        let at = HolePie::from_token(control);
        match open::grant(&at, TaskId::new(from), nonce) {
            Ok(reply) => {
                // 顶替旧会话：旧回复孔随 `close` 回收（那是本服务自己开的那一枚，不回收
                // 就是每重连一次漏一份）。
                if let Some(old) = self.slots[i] {
                    let _ = old.close();
                }
                // 三格：推者、本服务这侧的回复孔、**探针**。
                // 探针 = `at`（推者开的握手孔在本服务表里那枚副本）——它**不能放下**：
                // 放下就等于把"推者还在吗"这条判据丢了（`Session::probe` 只认对端开的那一枚）。
                let session = Session::new(
                    TaskId::new(from),
                    PieToken::new(reply.token()),
                    PieToken::new(at.token()),
                );
                self.slots[i] = Some(session);
                // **去向必须是新建会话的回信孔**：回执只有经它才到得了客户端。
                // （第一版写成 `None` = 丢弃 ⇒ 客户端每条请求都等到超时；实测过。）
                Outcome::say(Reply::Ok { client: i + 1 }, Some(session))
            }
            // 授不出去 = 对端已经走不动了：**不占槽**，也不回话（无处可回）。
            // 那枚握手孔留着没用（这条会话没开成）⇒ 当场放下。
            Err(_) => {
                let _ = at.release();
                Outcome::quiet()
            }
        }
    }

    /// 关一条会话。
    ///
    /// 到得了这里的请求**已经过了归属闸**（[`State::handle`]）：号在、且是这位推者的。
    /// 故这里不再判"认不认识"——那是另一种失败，已经在上面答过了。
    fn close(&mut self, client: usize) -> Outcome {
        // **先取会话，再清格**：清完再查表就没人知道这句回执该往哪儿推了——那条 `Ok`
        // 会被丢掉，客户端白等满上界（`session` 自检第一次跑就现形：`closed=0`）。
        let to_reply = self.take(client);
        Outcome {
            reply: Some(Reply::Ok { client }),
            to_reply,
            closing: true,
        }
    }

    /// 取走那条会话：**先取后清**，回执要用它。
    fn take(&mut self, client: usize) -> Option<Session> {
        let session = self.session(client)?;
        self.forget(session);
        Some(session)
    }

    /// 忘掉一条会话那一格（按**对端**认；一个推者至多一条会话）。
    ///
    /// 两个触发：`Close`（协议）、推不动（[`Session::push`] 报错 ⇒ 这条会话没有出口了）。
    /// 收场只清格——回复孔的 pie 由 [`Session::close`] 放，本函数不碰孔。
    pub fn forget(&mut self, session: Session) {
        let Some(i) = self
            .slots
            .iter()
            .position(|s| s.is_some_and(|s| s.peer() == session.peer()))
        else {
            return;
        };
        self.slots[i] = None;
        if self.reading.as_ref().is_some_and(|r| r.client == i + 1) {
            self.reading = None;
        }
    }

    /// 渲一帧：有会话在等读时**先擦当前行、打印、再重画它**。
    ///
    /// 字节只落进**出帧槽**（[`State::take_out`]），落屏由请求线程做——本层不认识设备。
    ///
    /// 回执即**同步点**：客户端收到 Ok 才知道这段已落屏。
    fn write(&mut self, client: usize, text: &[u8]) -> Outcome {
        match core::str::from_utf8(text) {
            Ok(s) => {
                // 正在编辑 → 先清行（否则消息会插进用户正在打的输入串中间）
                if self.reading.is_some() {
                    self.emit("\r\x1b[K");
                }
                self.emit(s);
                self.redraw();
                Outcome::say(Reply::Ok { client }, self.session(client))
            }
            // 非 UTF-8：逐字节写会把转义串打成碎片，故按"不可渲染"拒收。
            // 旧的 `io::put` 收 `&str`，调用方本来就给不出这种载荷。
            Err(_) => Outcome::say(Reply::Denied, self.session(client)),
        }
    }

    /// 读一行：**只登记**。整行由输入线程在回车/Ctrl-C/Ctrl-D 时交付。
    ///
    /// 提示符**由客户端先同步写过一次**（那条 `Write` 的 Ok 即"已落屏"）；
    /// 这里只把它登记下来供重绘使用——重绘是"清行后整行重写"，故不会重复显示。
    ///
    /// 到得了这里的请求**已经过了归属闸**（[`State::handle`]）：号在、且是这位推者的，
    /// 故这里不再判"认不认识"。
    fn readline(&mut self, client: usize, prompt: &[u8]) -> Outcome {
        // 已在等读：**只认"同一个会话重问同一条"**，其余照旧不排队（行编辑是单读者
        // 语义）。这条幂等是**给客户端留的退路**：客户端那边 `readline` 的等待必须
        // 有界（否则服务被打死而等待没被唤醒时，它就永久挂住——实测过），有界就得
        // 能在超时后**重发同一条**请求，而重发要被当成"还是那一问"而不是"第二问"。
        //
        // 只比 `client` 不比 `prompt`：prompt 是**重绘**用的参数，同一个会话的两条
        // `ReadLine` 里它必然相同；比它只是多加一处可以不同的地方。
        if let Some(r) = self.reading.as_ref() {
            if r.client == client {
                // 不回复：整行仍由输入线程交付（与首次那一条同样的走法）。
                return Outcome::quiet();
            }
            return Outcome::say(Reply::NoSuchClient, self.session(client));
        }
        let prompt = core::str::from_utf8(prompt).unwrap_or("");
        self.reading = Some(Reading {
            client,
            prompt: String::from(prompt),
            line: Line::default(),
        });
        Outcome::quiet()
    }

    /// 重画当前输入行（`\r\x1b[K` + prompt + 缓冲 + 光标定位）。
    fn redraw(&mut self) {
        let Some(r) = self.reading.as_ref() else {
            return;
        };
        // 先把整帧拼出来再落槽：`emit` 要 `&mut self`，而 `r` 还借着 `self.reading`。
        let back = (r.line.buf.len() - r.line.pos) as u16;
        let mut frame = String::from("\r\x1b[K");
        frame.push_str(&r.prompt);
        frame.push_str(&r.line.text());
        if back > 0 {
            frame.push_str(&format!("\x1b[{back}D"));
        }
        self.emit(&frame);
    }

    /// 有没有会话在等读（输入线程据此决定要不要从**投递孔**取字节）。
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
                    self.out.extend_from_slice(b"\r\n");
                }
                // Ctrl-C / Ctrl-D **也要换行**：这两个键结束的是一整行，若不换行，
                // 客户端下一轮的重绘是 `\r\x1b[K`（**回到本行行首**）⇒ 新提示符把
                // 上一个提示符原地盖掉，用户看到"Ctrl-C 之后没有新行"。
                // 回车那一支同理——三个收尾键在这一件事上必须一致。
                Key::Interrupt | Key::Eof => {
                    self.out.extend_from_slice(b"\r\n");
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
                let line = r.line.text();
                match Text::new(line.as_bytes()) {
                    Some(text) => Reply::Line { text },
                    // 行长超过 `LINE` 是服务侧不可能造出的状态（行缓冲有自己的上限）；
                    // 真撞上就按"这一行没了"回，不截断。
                    None => Reply::Eof,
                }
            }
        };
        Some((r.client, reply))
    }

    /// 那条会话（**本线程表里**那三格）。
    ///
    /// 两个消费者：输入线程交付整行时经它取出孔（那个任务推不动孔，只有主线程推得动）；
    /// 以及本条回复的落点（[`Outcome::to_reply`]）。
    pub fn session(&self, client: usize) -> Option<Session> {
        let i = self.index(client)?;
        self.slots[i]
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
