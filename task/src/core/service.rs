//! Service 域：服务目录协议客户端（`Directory` 会话 + `Service` 句柄）。
//!
//! 协议规范见 `docs/dispatch.md`。三条要点：
//!
//! 1. **内核没有目录入口调用**（class 7 已删）：内核在 boot 期把一枚 pie 放进本
//!    任务权限表——索引 0 = 目录入口门闩——`Directory::open` 用 `Collect` 取回，
//!    顺带拿到它的 `vestor` = 目录 task id。故无需向用户态传任何整数。
//! 2. **回信通道由调用方自带**：`UnsealHole` 造自己的 hole，`Accord` 给目录，把
//!    对端侧 token 写进请求 `[49..57]`——目录按这枚 pie 的 `vestor` 认人。
//!    服务调用同理，token 放在消息前 8 字节（owner 由 `Connect` 回复给出）。
//! 3. **服务调用载荷 56 字节**（`MSG_LEN` - 8 字节回信 token）。
//!
//! 与 [`crate::core::channel::Channel`] 同住 `core/`（envcall 转发外的封装层）。

use env::dispatch::{MSG_LEN, Name, Reply, Request};
use env::{EnvError, EnvResult, TaskId, make_err};

use crate::env::mail::{self, HolePie};

use super::channel::Channel;

/// 服务调用载荷字节数（`MSG_LEN` - 8 字节回信 token）。
pub const PAYLOAD_LEN: usize = MSG_LEN - 8;

/// 等回复的上界（毫秒）。目录/服务往返都在本域内，1s 远超实际耗时；有上界才能
/// 把「回复被丢弃」这类协议错误暴露成 `Busy`，而不是永久挂起。
const REPLY_TIMEOUT_MS: usize = 1000;

/// D1 负码：无权 / 协议错。
const E_DENIED: isize = -1;
/// D1 负码：名字无绑定。
const E_NOT_FOUND: isize = -2;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_DENIED))
}

fn parse_name(name: &str) -> EnvResult<Name> {
    Name::new(name).map_err(|_| denied())
}

/// 目录会话：入口门闩 + 自造 reply（Accord 给目录作 per-caller 通道）。
pub struct Directory {
    entry: HolePie,
    reply: HolePie,
    /// reply hole 在目录侧的 token——写进请求 `[49..57]`，目录按此推回复。
    reply_target: u64,
}

impl Directory {
    /// 打开会话：从本任务权限表取回内核在 boot 期放下的目录入口门闩（索引 0），
    /// 自造 reply hole、`Accord` 给目录，记下对端 token 供每条请求回填。
    ///
    /// **per-caller reply**：reply hole 由调用方自备；目录 `[49..57]` 字段就是
    /// reply_target。`Collect` 顺带返 `vestor = dir_id`，直接当 `Accord` 的 dst。
    pub fn open() -> EnvResult<Directory> {
        let (entry_tok, _entry_perm, dir_id) = mail::collect(0)?;
        if entry_tok == 0 {
            return Err(denied());
        }
        if dir_id.get() == 0 {
            return Err(denied()); // 入口 pie 没标宿主（vestor=None），无法 Accord
        }
        // 自造 reply：unseal + accord(dir_id)——目录侧那枚 token 即本会话的回信地址。
        let reply_mine = HolePie::unseal(crate::env::mail::HOLE_MTU_MAX)?;
        let reply_target =
            reply_mine.accord(dir_id.get(), env::Permission::READ | env::Permission::WRITE)?;
        Ok(Directory {
            entry: HolePie::from_token(entry_tok),
            reply: reply_mine,
            reply_target,
        })
    }

    /// 一次往返：push 请求（带 reply token）→ 有界 pull 回复。
    ///
    /// 超时（`Busy`）后本会话不可复用——迟到的回复会污染下一次 pull。
    fn call(&self, request: &Request) -> EnvResult<Reply> {
        let mut msg = request.encode();
        msg[env::dispatch::REPLY_AT..env::dispatch::REPLY_AT + 8]
            .copy_from_slice(&self.reply_target.to_le_bytes());
        self.entry.push(&msg)?;
        let mut buf = [0u8; MSG_LEN];
        self.reply.pull_timeout(&mut buf, REPLY_TIMEOUT_MS)?;
        Reply::decode(&buf).map_err(|_| denied())
    }

    /// 纯探测：这个名字有没有绑定。
    pub fn discover(&self, name: &str) -> EnvResult<bool> {
        let request = Request::Resolve {
            name: parse_name(name)?,
        };
        match self.call(&request)? {
            Reply::Found { .. } => Ok(true),
            Reply::NotFound => Ok(false),
            _ => Err(denied()),
        }
    }

    /// 枚举下一页：按名字排序；`after = None` 从头开始；`None` 返回到头。
    pub fn list(&self, after: Option<&str>) -> EnvResult<Option<Name>> {
        let after = match after {
            Some(s) => Some(parse_name(s)?),
            None => None,
        };
        let request = Request::Enumerate { after };
        match self.call(&request)? {
            Reply::Found { name } => Ok(Some(name)),
            Reply::NotFound => Ok(None),
            _ => Err(denied()),
        }
    }

    /// 连接：目录把服务的入口门闩转授给本任务，并回 owner task id。
    pub fn connect(&self, name: &str) -> EnvResult<Service> {
        let request = Request::Connect {
            name: parse_name(name)?,
        };
        match self.call(&request)? {
            Reply::Connected { entry, owner } => {
                let channel = Channel::open(owner)?;
                Ok(Service {
                    entry: HolePie::from_token(entry.get()),
                    channel,
                    owner,
                })
            }
            Reply::NotFound => Err(make_err(EnvError::from_raw(E_NOT_FOUND))),
            _ => Err(denied()),
        }
    }
}

/// 服务句柄：入口门闩 + 回信通道（调用方自带，故并发调用不串台）。
pub struct Service {
    entry: HolePie,
    channel: Channel,
    owner: TaskId,
}

impl Service {
    /// 一次调用：前 8 字节自动填回信 token，载荷 `PAYLOAD_LEN` 字节。
    /// 回复有上界（`REPLY_TIMEOUT_MS`）——服务漏回即 `Busy`，不永久挂起。
    pub fn call(&self, payload: &[u8; PAYLOAD_LEN]) -> EnvResult<[u8; PAYLOAD_LEN]> {
        let mut msg = [0u8; MSG_LEN];
        msg[0..8].copy_from_slice(&self.channel.at_peer.to_le_bytes());
        msg[8..].copy_from_slice(payload);
        self.entry.push(&msg)?;

        let mut buf = [0u8; MSG_LEN];
        self.channel.mine.pull_timeout(&mut buf, REPLY_TIMEOUT_MS)?;
        let mut out = [0u8; PAYLOAD_LEN];
        out.copy_from_slice(&buf[8..]);
        Ok(out)
    }

    /// 断开：撤回服务侧的回信副本 + 放下入口门闩。目录不记连接状态，故到此为止。
    pub fn disconnect(self) -> EnvResult<()> {
        self.channel.close(self.owner)?;
        self.entry.release()
    }
}
