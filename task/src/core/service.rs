//! Service 域：服务目录协议客户端（`Directory` 会话 + `Service` 句柄）。
//!
//! 协议规范见 `docs/dispatch.md`。四条要点：
//!
//! 1. **内核没有目录入口调用**（class 7 已删）：目录请求门闩由**父域经启动期握手
//!    配给**（`handshake::Pier` 的载荷），`Directory::open` 直接收下这枚句柄。
//! 2. **对端是谁由 `Owned` 求得**：目录 id = `Owned(entry).owner`（资源开辟者），
//!    服务 id 同理——`vestor` 会被转发改写成 root，`owner` 不会。
//! 3. **身份由内核盖章**：目录认的调用方 = `Push` 时内核写进槽的发送者。回信通道
//!    由调用方自带（`UnsealHole` + `Accord` 给目录），token 写进请求 `[49..57]`。
//!    服务调用同理，token 放在消息前 8 字节。
//! 4. **服务调用载荷 56 字节**（`MSG_LEN` - 8 字节回信 token）。
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
pub const E_DENIED: isize = -1;
/// D1 负码：名字无实例 / 未预约。
pub const E_NOT_FOUND: isize = -2;
/// D1 负码：名字已有活实例（内核码 -1..-6 之后自取）。
pub const E_TAKEN: isize = -7;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_DENIED))
}

fn not_found() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_NOT_FOUND))
}

fn taken() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_TAKEN))
}

/// 服务交给目录保管入口门闩时的权限：读写 + **转授**（目录要能再授给客户端）。
fn entry_permission() -> env::Permission {
    env::Permission::READ | env::Permission::WRITE | env::Permission::VEST
}

fn parse_name(name: &str) -> EnvResult<Name> {
    Name::new(name).map_err(|_| denied())
}

/// 目录会话：入口门闩 + 自造 reply（Accord 给目录作 per-caller 通道）。
pub struct Directory {
    entry: HolePie,
    reply: HolePie,
    /// reply hole 在目录侧的 token——写进请求 `[49..57]`，目录按此推回复。
    reply_target: usize,
    /// 目录 task id（`Owned(entry).owner`）——发布时把入口门闩 `Accord` 给它。
    dir_id: usize,
}

impl Directory {
    /// 打开会话：`entry` = 父域经启动期握手配给的目录请求门闩（`Pier` 的载荷）。
    /// 自造 reply hole、`Accord` 给目录，记下对端 token 供每条请求回填。
    ///
    /// **per-caller reply**：reply hole 由调用方自备；目录 `[49..57]` 字段就是
    /// reply_target。目录 task id 取 `Owned(entry).owner`——门闩的**开辟者**；
    /// root 转发给本任务时 `vestor` 已变成 root，只有 `owner` 还指着目录。
    pub fn open(entry: HolePie) -> EnvResult<Directory> {
        let dir_id = mail::owned(entry.token())?.1.get();
        if dir_id == 0 {
            return Err(denied());
        }
        // 自造 reply：unseal + accord(dir_id)——目录侧那枚 token 即本会话的回信地址。
        let reply_mine = HolePie::unseal(crate::env::mail::HOLE_MTU_MAX)?;
        let reply_target =
            reply_mine.accord(dir_id, env::Permission::READ | env::Permission::WRITE)?;
        Ok(Directory {
            entry,
            reply: reply_mine,
            reply_target,
            dir_id,
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

    /// 本会话在**目录侧**的回信 token（诊断 / 自检用：写进请求 `[49..57]`）。
    pub fn reply_target(&self) -> usize {
        self.reply_target
    }

    /// 发一条请求并只认 `Ok`。
    fn ack(&self, request: Request) -> EnvResult<()> {
        match self.call(&request)? {
            Reply::Ok => Ok(()),
            Reply::NotFound => Err(not_found()),
            Reply::Taken => Err(taken()),
            _ => Err(denied()),
        }
    }

    /// 注册：把入口门闩交给目录保管（`Accord` 给目录，带 `VEST`——目录要能再转授）
    /// 并登记名字。名字必须已由**父域预约**给本任务，否则 `NotFound` / `Denied`。
    pub fn register(&self, name: &str, entry: &HolePie) -> EnvResult<()> {
        let target = entry.accord(self.dir_id, entry_permission())?;
        self.ack(Request::Register {
            name: parse_name(name)?,
            entry: env::PieToken::new(target),
        })
    }

    /// 注销：摘掉实例；名字仍归本任务（预约行保留），可再次注册。
    pub fn unregister(&self, name: &str) -> EnvResult<()> {
        self.ack(Request::Unregister {
            name: parse_name(name)?,
        })
    }

    /// 换绑：服务换了入口门闩，名字不变（覆盖旧实例）。
    pub fn replace(&self, name: &str, entry: &HolePie) -> EnvResult<()> {
        let target = entry.accord(self.dir_id, entry_permission())?;
        self.ack(Request::Replace {
            name: parse_name(name)?,
            entry: env::PieToken::new(target),
        })
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

    /// 连接：目录把服务的入口门闩转授给本任务；服务 id 由 `Owned` 从门闩求得。
    pub fn connect(&self, name: &str) -> EnvResult<Service> {
        let request = Request::Connect {
            name: parse_name(name)?,
        };
        match self.call(&request)? {
            Reply::Connected { entry } => {
                let owner = mail::owned(entry.get())?.1;
                let channel = Channel::open(owner)?;
                Ok(Service {
                    entry: HolePie::from_token(entry.get()),
                    channel,
                    owner,
                })
            }
            Reply::NotFound => Err(not_found()),
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
