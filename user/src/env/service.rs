//! Service 域：服务目录协议客户端（`Directory` 会话 + `Service` 句柄）。
//!
//! 协议规范见 `docs/dispatch.md`。三条要点：
//!
//! 1. **内核没有目录入口调用**（class 7 已删）：内核在 boot 期把两枚 pie 放进本
//!    任务权限表——索引 0 = 目录入口门闩，索引 1 = 回信 hole——`Directory::open`
//!    用 `Collect` 取回。故无需向用户态传任何整数。
//! 2. **服务侧回信通道由调用方自带**：`UnsealHole` 造自己的 hole，`Accord` 委托给
//!    服务（owner 由 `Connect` 回复给出），请求消息前 8 字节放那个对端侧 token。
//! 3. **服务调用载荷 56 字节**（`MSG_LEN` - 8 字节回信 token）。

use ubi::dispatch::{MSG_LEN, Name, Reply, Request};
use ubi::{EnvError, EnvResult, Permission, TaskId, make_err};

use super::mail::{self, HolePie};

/// 服务调用载荷字节数（`MSG_LEN` - 8 字节回信 token）。
pub const PAYLOAD_LEN: usize = MSG_LEN - 8;

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

/// 回信通道：我自己的 hole + 它在对端的 token。
struct Channel {
    mine: HolePie,
    at_peer: u64,
}

impl Channel {
    /// 建通道：`UnsealHole` 造自己的 hole，`Accord` 把 R|W 委托给对端。
    fn open(peer: TaskId) -> EnvResult<Channel> {
        let mine = HolePie::unseal()?;
        let at_peer = mine.accord(peer.get(), Permission::READ | Permission::WRITE)?;
        Ok(Channel { mine, at_peer })
    }

    /// 关闭：撤回委托给对端的那一份 + 放下自己的 hole。
    fn close(self, peer: TaskId) -> EnvResult<()> {
        mail::revoke(peer.get(), self.at_peer)?;
        self.mine.release()
    }
}

/// 目录会话：入口门闩（索引 0）+ 内核预置的回信 hole（索引 1）。
pub struct Directory {
    entry: HolePie,
    reply: HolePie,
}

impl Directory {
    /// 打开会话：从本任务权限表取回内核在 boot 期放下的两枚 pie。
    ///
    /// 约定：索引 0 = 目录入口门闩，索引 1 = 回信 hole（`boot::spawn_demos` 的
    /// 根授予顺序）。两枚 pie 都在 boot 期间落表（此时无任务在跑），故无需等待。
    pub fn open() -> EnvResult<Directory> {
        let (entry, _) = mail::collect(0)?;
        let (reply, _) = mail::collect(1)?;
        if entry == 0 || reply == 0 {
            return Err(denied());
        }
        Ok(Directory {
            entry: HolePie::from_token(entry),
            reply: HolePie::from_token(reply),
        })
    }

    /// 一次往返：push 请求 → pull 回复。
    fn call(&self, request: &Request) -> EnvResult<Reply> {
        self.entry.push(&request.encode())?;
        let mut buf = [0u8; MSG_LEN];
        self.reply.pull(&mut buf)?;
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
    pub fn call(&self, payload: &[u8; PAYLOAD_LEN]) -> EnvResult<[u8; PAYLOAD_LEN]> {
        let mut msg = [0u8; MSG_LEN];
        msg[0..8].copy_from_slice(&self.channel.at_peer.to_le_bytes());
        msg[8..].copy_from_slice(payload);
        self.entry.push(&msg)?;

        let mut buf = [0u8; MSG_LEN];
        self.channel.mine.pull(&mut buf)?;
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
