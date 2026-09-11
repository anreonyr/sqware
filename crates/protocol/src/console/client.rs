//! console·client — 线对侧：`Console` 会话 + `Readline` 结果。
//!
//! 与旧 `programs/src/term` 的 `Terminal`/`Readline` **同形**，但内部改走协议：
//! 客户端不再碰 UART，只往请求孔推请求、从自己的回信孔收回复。
//!
//! # 同步点在哪
//!
//! `write` **等到 Ok 才返回**——这条很要紧：`write(prompt)` 返回时提示符已落屏，
//! 之后的 `readline` 才能安全地等输入。若 `write` 是"推完就算"，提示符会憋在孔里，
//! 而用户已经在对空行打字。
//!
//! 这正是 dispatch 的 `Channel` + `pull_timeout` 的成熟形，不新造机制。
//!
//! # 生命周期契约（显式，不靠 Drop）
//!
//! `HolePie` 没有 `Drop`——它是**句柄值**，不是 RAII 守卫。故本会话的资源由
//! [`Console::close`] 显式放：会话登记（服务侧）随 `Close` 撤回，两枚孔随进程退出
//! 由内核回收。**不假装**有 `Drop`——那是这套机制既有的形状，本模块照它写。

use alloc::string::String;

use env::{EnvError, EnvResult, PieToken, make_err};
use runtime::core::channel::Channel;
use runtime::env::mail::{self, HolePie};

use super::wire::{MSG_LEN, PAYLOAD_LEN, Reply, Request};

/// 一次往返的上界（毫秒）。与 dispatch 的 `REPLY_TIMEOUT_MS` 同值。
const REPLY_TIMEOUT_MS: usize = 1000;


fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 读一行的结果。与旧 `term::Readline` 同形（`Line`/`Eof`/`Interrupt`）。
#[derive(Debug)]
pub enum Readline {
    /// 一行文本（回车提交，不含 `\n`）。
    Line(String),
    /// Ctrl-D。
    Eof,
    /// Ctrl-C。
    Interrupt,
}

/// 控制台会话：请求门闩 + 回信孔（+ 会话 id）。
///
/// `entry` 由**父域经启动期握手配给**（与目录入口同一套 `Pier`），之后一切走本会话。
pub struct Console {
    /// 请求门闩：往它 push 请求。
    entry: HolePie,
    /// 会话 id（0 = 未开成）。
    client: usize,
    /// 回信孔：我这一侧的句柄 + 它在服务侧的号。
    ///
    /// `Channel` 只是"我这枚 `HolePie` + 对端那枚的号"的打包，方便 `open` 一次配好；
    /// **孔本身与它绑不绑没有关系**——`HolePie` 没有 `Drop`，`from_token` 是零成本
    /// 重建（这一点曾判错，把"孔会随句柄消亡"当成了根因；真正的根因是
    /// **交错了 token**：必须交 `at_peer`，且 `push` 只在**推者自己的表**里查）。
    reply: Channel,
}

impl Console {
    /// 打开会话：自建**回信孔**一枚，`Accord` 给服务，然后 `Open`（把它的对端句柄交出去）。
    ///
    /// 对端 task id 由 `Owned(entry).owner` 求得——**门闩的开辟者就是服务本身**
    /// （root 转发不改 `owner`，只改 `vestor`）。
    pub fn open(entry: HolePie) -> EnvResult<Console> {
        let owner = mail::reserve(PieToken::new(entry.token()))?.1.get();
        if owner == 0 {
            return Err(denied());
        }
        // **一条**通道就够：回信孔。请求走 `entry`（请求门闩），故不需要数据孔
        // （第一版多开的那枚从没被读过，已删）。
        let reply = Channel::open(env::TaskId::new(owner))?;
        let mut console = Console {
            entry,
            client: 0,
            reply,
        };
        // 交出去的必须是 `at_peer`（连踩两次的第一处）：服务 `push` 时按
        // `find(token)` 在**它自己的表**里找，`at_peer` 正是 `Accord` 给它那枚的号；
        // 交 `mine`（我表里的号）→ `Denied`。第二处在**服务侧**：整行必须由持有这枚
        // token 的那个 task 推。
        let ack = console.call(&Request::Open {
            reply: console.reply.at_peer().get(),
        })?;
        match ack {
            Reply::Ok { client } => {
                console.client = client;
                Ok(console)
            }
            _ => Err(denied()),
        }
    }

    /// 本会话 id（0 = 未开成）。
    pub fn client(&self) -> usize {
        self.client
    }

    /// 一次往返：请求推请求孔、回复从自己的回信孔**有界**收。
    fn call(&self, request: &Request) -> EnvResult<Reply> {
        self.entry.push(&request.encode())?;
        let mut buf = [0u8; MSG_LEN];
        self.reply.mine().pull_timeout(&mut buf, REPLY_TIMEOUT_MS)?;
        Reply::decode(&buf).map_err(|_| denied())
    }

    /// 写字符串：**阻塞到服务确认落屏**。
    ///
    /// 按 [`PAYLOAD_LEN`] 分片；每片一次往返。旧 `io::put` 是同步写设备，
    /// 故这里的同步语义与它同级——`write(prompt)` 返回即可安全开始读。
    pub fn write(&self, s: &str) -> EnvResult<()> {
        if self.client == 0 {
            return Err(denied());
        }
        for chunk in s.as_bytes().chunks(PAYLOAD_LEN) {
            let mut payload = [0u8; PAYLOAD_LEN];
            payload[..chunk.len()].copy_from_slice(chunk);
            match self.call(&Request::Write {
                client: self.client,
                len: chunk.len(),
                payload,
            })? {
                Reply::Ok { .. } => {}
                _ => return Err(denied()),
            }
        }
        Ok(())
    }

    /// 读一行（带行编辑）。**无上界**——用户在打字，服务端阻塞等回车。
    ///
    /// `prompt` 随请求带上（服务侧重绘要用它）：本行先把它同步写到设备上，
    /// 再请求读行——两件事的次序不能颠倒（否则用户对着空行打字）。
    pub fn readline(&self, prompt: &str) -> EnvResult<Readline> {
        if self.client == 0 {
            return Err(denied());
        }
        if !prompt.is_empty() {
            self.write(prompt)?;
        }
        let bytes = prompt.as_bytes();
        let n = if bytes.len() > PAYLOAD_LEN { PAYLOAD_LEN } else { bytes.len() };
        let mut pbuf = [0u8; PAYLOAD_LEN];
        pbuf[..n].copy_from_slice(&bytes[..n]);
        self.entry.push(
            &Request::ReadLine {
                client: self.client,
                len: n,
                prompt: pbuf,
            }
            .encode(),
        )?;
        let mut buf = [0u8; MSG_LEN];
        // 阻塞读（不是 `pull_timeout`）：等多久由用户决定。
        self.reply.mine().pull(&mut buf)?;
        match Reply::decode(&buf).map_err(|_| denied())? {
            Reply::Line { len, payload } => {
                let n = if len > PAYLOAD_LEN { PAYLOAD_LEN } else { len };
                let text = core::str::from_utf8(&payload[..n]).map_err(|_| denied())?;
                Ok(Readline::Line(String::from(text)))
            }
            Reply::Eof => Ok(Readline::Eof),
            Reply::Interrupt => Ok(Readline::Interrupt),
            _ => Err(denied()),
        }
    }

    /// 关会话：撤回服务侧的会话登记。
    ///
    /// 两枚孔的副本随进程退出由内核回收（`HolePie` 不是 RAII 守卫，见文件头）。
    pub fn close(&self) -> EnvResult<()> {
        if self.client == 0 {
            return Ok(());
        }
        match self.call(&Request::Close { client: self.client })? {            Reply::Ok { .. } => Ok(()),
            _ => Err(denied()),
        }
    }
}
