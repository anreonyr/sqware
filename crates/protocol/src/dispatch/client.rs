//! Service 域：服务目录协议客户端（`Directory` 会话 + `Service` 句柄）。
//!
//! 协议规范见 `docs/dispatch.md`。四条要点：
//!
//! 1. **内核没有目录入口调用**（class 7 已删）：目录请求门闩由**父域经启动期握手
//!    配给**（`handshake::Pier` 的载荷），`Directory::open` 直接收下这枚句柄。
//! 2. **对端是谁由 `Owned` 求得**：目录 id = `Reserve(entry).owner`（资源开辟者），
//!    服务 id 同理——`vestor` 会被转发改写成 root，`owner` 不会。
//! 3. **身份由内核盖章**：目录认的调用方 = `Push` 时内核写进槽的发送者。回信通道
//!    由调用方自带（一枚 Hole + 授出的那份），地址写进请求 `[49..57]`。服务调用同理，
//!    地址放在消息前 8 字节。
//! 4. **服务调用载荷 56 字节**（`MSG_LEN` - 8 字节回信 token）。
//!
//! 本模块用 `runtime` 的机制：门闩原语（`runtime::env::mail`）与往返
//! （[`runtime::core::port::Port`]）。协议层**不碰** `env::ecall`。

use env::{EnvResult, PieToken, TaskId};

use runtime::core::port::{Access, Duet, Policy, Port, ship};
use runtime::env::mail::{self, AnyPie as _, HolePie};

use super::wire::{MSG_LEN, Name, Reply, Request, denied, not_found, taken};

/// 服务调用载荷字节数（`MSG_LEN` - 8 字节回信 token）。
pub const PAYLOAD_LEN: usize = MSG_LEN - 8;

/// 等回复的上界（毫秒）。目录/服务往返都在本域内，1s 远超实际耗时；有上界才能
/// 把「回复被丢弃」这类协议错误暴露成 `Busy`，而不是永久挂起。
const REPLY_TIMEOUT_MS: usize = 1000;

/// 服务调用报文的字节数（与目录请求同尺寸：同一代协议）。
const CALL_LEN: usize = MSG_LEN;

fn parse_name(name: &str) -> EnvResult<Name> {
    Name::new(name).map_err(|_| denied())
}

/// 目录会话：请求门闩 + 自造 reply（授给目录作 per-caller 通道）。
///
/// 与 [`super::server::Directory`] **同名不同角色**，读到先看模块：本枚是**客户端**
/// （跟目录说话），那一枚是**服务端**（目录自己那张预约表）。两者是同一份协议的两侧。
pub struct Directory {
    port: Port,
    /// 目录 task id（`Reserve(entry).owner`）——发布时把入口门闩授给它。
    dir_id: TaskId,
}

impl Directory {
    /// 打开会话：`entry` = 父域经启动期握手配给的目录请求门闩（`Pier` 的载荷）。
    ///
    /// 目录 task id 取 `Reserve(entry).owner`——门闩的**开辟者**；root 转发给本任务时
    /// `vestor` 已变成 root，只有 `owner` 还指着目录。回信孔与它的授出归 [`Port::open`]。
    pub fn open(entry: HolePie) -> EnvResult<Directory> {
        let dir_id = mail::reserve(PieToken::new(entry.token()))?.1;
        if dir_id.get() == 0 {
            return Err(denied());
        }
        Ok(Directory {
            port: Port::open(&entry)?,
            dir_id,
        })
    }

    /// 一次往返：五步（填回信地址 / push / 校来源 / 有界等 / 解码）都在 [`Port::call`]。
    ///
    /// 超时（`Busy`）或来源不符（`Denied`）后本会话不可复用——迟到的回复会污染下一次。
    fn call(&self, request: &Request) -> EnvResult<Reply> {
        self.port.call::<Request>(request, REPLY_TIMEOUT_MS)
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

    /// 注册：把入口门闩交给目录保管（授出**读写 + 转授**——目录要能再授给客户端）
    /// 并登记名字。名字必须已由**父域预约**给本任务，否则 `NotFound` / `Denied`。
    pub fn register(&self, name: &str, entry: &HolePie) -> EnvResult<()> {
        let to = ship(
            entry,
            self.dir_id,
            Access::READ | Access::WRITE,
            Policy::VEST,
        )?;
        self.ack(Request::Register {
            name: parse_name(name)?,
            entry: to.seed(),
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
        let to = ship(
            entry,
            self.dir_id,
            Access::READ | Access::WRITE,
            Policy::VEST,
        )?;
        self.ack(Request::Replace {
            name: parse_name(name)?,
            entry: to.seed(),
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

    /// 取某个已注册服务的**入口门闩 token**，不建回信通道。
    ///
    /// 与 [`Directory::connect`] 的分工：那个给"用服务调用协议说话"的调用方
    /// （要通道）；这个给"用**自己的协议**说话"的调用方（只要入口——如控制台）。
    /// 后者若先 `connect` 再 `disconnect`，会顺手 release 掉刚拿到的入口——
    /// 那是实测踩过的坑。
    pub fn connect_token(&self, name: &str) -> EnvResult<PieToken> {
        let request = Request::Connect {
            name: parse_name(name)?,
        };
        match self.call(&request)? {
            Reply::Connected { entry } => Ok(entry),
            Reply::NotFound => Err(not_found()),
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
                let entry = HolePie::from_token(entry.get());
                Ok(Service {
                    port: Port::open(&entry)?,
                    entry,
                })
            }
            Reply::NotFound => Err(not_found()),
            _ => Err(denied()),
        }
    }
}

/// 服务句柄：一次往返（[`Port`]）+ 入口门闩（收场时要放下它）。
pub struct Service {
    port: Port,
    entry: HolePie,
}

/// 服务调用协议**没有自己的请求类型**：载荷是不透明的 56 字节，前 8 字节是回信地址。
/// 故报文对的形状就落在句柄自己身上——两端同形，中间不夹一个只为转手的类型。
impl Duet for Service {
    type Req = [u8; PAYLOAD_LEN];
    type Rep = [u8; PAYLOAD_LEN];
    type Wire = [u8; CALL_LEN];

    const REQ: usize = CALL_LEN;
    const REP: usize = CALL_LEN;

    fn wire() -> [u8; CALL_LEN] {
        [0u8; CALL_LEN]
    }

    fn encode(req: &[u8; PAYLOAD_LEN], at: PieToken, out: &mut [u8]) {
        out[..8].copy_from_slice(&at.get().to_le_bytes());
        out[8..CALL_LEN].copy_from_slice(req);
    }

    fn decode(buf: &[u8]) -> EnvResult<[u8; PAYLOAD_LEN]> {
        let mut out = [0u8; PAYLOAD_LEN];
        out.copy_from_slice(buf.get(8..CALL_LEN).ok_or_else(denied)?);
        Ok(out)
    }
}

impl Service {
    /// 一次调用：前 8 字节的回信地址由 `Port` 填，载荷 `PAYLOAD_LEN` 字节。
    /// 回复有上界（`REPLY_TIMEOUT_MS`）——服务漏回即 `Busy`，不永久挂起。
    pub fn call(&self, payload: &[u8; PAYLOAD_LEN]) -> EnvResult<[u8; PAYLOAD_LEN]> {
        self.port.call::<Service>(payload, REPLY_TIMEOUT_MS)
    }

    /// 断开：放下回信孔 + 放下入口门闩。目录不记连接状态，故到此为止。
    ///
    /// 两件事各算各的：`close` 失败**不阻断**入口的释放（旧 `Channel` 那条 `?` 会一起跳过）。
    pub fn disconnect(self) -> EnvResult<()> {
        let closed = self.port.close();
        let released = self.entry.release();
        closed?;
        released
    }
}
