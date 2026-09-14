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
//! 这正是「一次往返」的成熟形（[`Port`]），不新造机制。
//!
//! # 生命周期契约（显式，不靠 Drop）
//!
//! `HolePie` 没有 `Drop`——它是**句柄值**，不是 RAII 守卫。故本会话的资源由
//! [`Console::close`] 显式放：会话登记（服务侧）随 `Close` 撤回，两枚孔随进程退出
//! 由内核回收。**不假装**有 `Drop`——那是这套机制既有的形状，本模块照它写。

use alloc::string::String;

use env::EnvResult;

use runtime::core::port::Port;
use runtime::env::mail::HolePie;

use super::wire::{PAYLOAD_LEN, Reply, Request, denied};

/// 一次往返的上界（毫秒）。与 dispatch 的 `REPLY_TIMEOUT_MS` 同值。
const REPLY_TIMEOUT_MS: usize = 1000;

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

/// 控制台会话：一次往返的机制（[`Port`]）+ 会话 id。
///
/// `entry` 由**父域经启动期握手配给**（与目录入口同一套 `Pier`），之后一切走本会话。
pub struct Console {
    port: Port,
    /// 会话 id（0 = 未开成）。
    client: usize,
}

impl Console {
    /// 打开会话：开一条往返（自建回信孔、把它授给服务）+ `Open`（回信地址随报文交出）。
    ///
    /// 对端 task id 由 `Port::open` 用 `Reserve(entry).owner` 求得——**门闩的开辟者就是
    /// 服务本身**（root 转发不改 `owner`，只改 `vestor`）。
    pub fn open(entry: HolePie) -> EnvResult<Console> {
        let port = Port::open(&entry)?;
        let mut console = Console { port, client: 0 };
        match console.call(&Request::open())? {
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

    /// 一次往返：请求推请求孔、回复从自己的回信孔**有界**收、**核来源**。
    fn call(&self, request: &Request) -> EnvResult<Reply> {
        self.port.call::<Request>(request, REPLY_TIMEOUT_MS)
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
        let n = if bytes.len() > PAYLOAD_LEN {
            PAYLOAD_LEN
        } else {
            bytes.len()
        };
        let mut pbuf = [0u8; PAYLOAD_LEN];
        pbuf[..n].copy_from_slice(&bytes[..n]);
        let request = Request::ReadLine {
            client: self.client,
            len: n,
            prompt: pbuf,
        };
        // 上界给 `usize::MAX`（永久）：等多久由用户决定。
        match self.port.call::<Request>(&request, usize::MAX)? {
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
        match self.call(&Request::Close {
            client: self.client,
        })? {
            Reply::Ok { .. } => Ok(()),
            _ => Err(denied()),
        }
    }
}
