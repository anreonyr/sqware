//! Service 域：服务目录协议客户端（`Directory` 会话 + `Service` 句柄）。
//!
//! 协议规范见 `docs/dispatch.md`。四条要点：
//!
//! 1. **内核没有目录入口调用**（class 7 已删）：目录请求门闩由**父域经启动期握手
//!    配给**（`crate::startup::Pier` 的载荷），`Directory::open` 直接收下这枚句柄。
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

use runtime::core::port::{Access, Policy, Port, ship};
use runtime::env::mail::{self, AnyPie as _, HolePie};

use super::wire::{ADDRESS_LEN, CAP, Name, Query, Reply, denied, not_found, put_address, taken};

/// 服务调用报文的字节数：地址槽 8 + 载荷 56。
pub const CALL: usize = 8 + PAYLOAD_LEN;

/// 服务调用载荷字节数。
///
/// 56 是**留给载荷的上界**（`64 - 地址槽 8`）：服务调用的载荷不透明，宽一点不亏——
/// 它与目录询问的容量（`wire::CAP = 48`）不是同一个数，两者各有各的算法。
pub const PAYLOAD_LEN: usize = 56;

/// 等回复的上界（毫秒）。目录/服务往返都在本域内，1s 远超实际耗时；有上界才能
/// 把「回复被丢弃」这类协议错误暴露成 `Busy`，而不是永久挂起。
const REPLY_TIMEOUT_MS: usize = 1000;

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

    /// 本协议的一次往返：编帧（回信地址按**本协议**那一格填）→ 推 → 收（[`Port::pull`]
    /// 内建核对来源）→ 解码。
    ///
    /// 超时（`Busy`）或来源不符（`Denied`）后本会话不可复用——迟到的回复会污染下一次。
    fn call(&self, query: &Query) -> EnvResult<Reply> {
        let (frame, n) = query.encode(self.port.seed());
        self.port.push(frame.get(..n).ok_or_else(denied)?)?;
        let mut out = [0u8; CAP];
        let buf = self.port.pull(&mut out, REPLY_TIMEOUT_MS)?;
        Reply::decode(buf).map_err(|_| denied())
    }

    /// 发一条请求并只认 `Ok`。
    fn ack(&self, query: Query) -> EnvResult<()> {
        match self.call(&query)? {
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
        self.ack(Query::Register {
            name: parse_name(name)?,
            entry: to.seed(),
        })
    }

    /// 注销：摘掉实例；名字仍归本任务（预约行保留），可再次注册。
    pub fn unregister(&self, name: &str) -> EnvResult<()> {
        self.ack(Query::Unregister {
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
        self.ack(Query::Replace {
            name: parse_name(name)?,
            entry: to.seed(),
        })
    }

    /// 纯探测：这个名字有没有绑定。
    pub fn discover(&self, name: &str) -> EnvResult<bool> {
        let query = Query::Resolve {
            name: parse_name(name)?,
        };
        match self.call(&query)? {
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
        let query = Query::Enumerate { after };
        match self.call(&query)? {
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
        let query = Query::Connect {
            name: parse_name(name)?,
        };
        match self.call(&query)? {
            Reply::Connected { entry } => Ok(entry),
            Reply::NotFound => Err(not_found()),
            _ => Err(denied()),
        }
    }

    /// 连接：目录把服务的入口门闩转授给本任务；服务 id 由 `Owned` 从门闩求得。
    pub fn connect(&self, name: &str) -> EnvResult<Service> {
        let query = Query::Connect {
            name: parse_name(name)?,
        };
        match self.call(&query)? {
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

impl Service {
    /// 一次调用：前 8 字节的回信地址由本协议填，载荷 `PAYLOAD_LEN` 字节。
    /// 回复有上界（`REPLY_TIMEOUT_MS`）——服务漏回即 `Busy`，不永久挂起。
    pub fn call(&self, payload: &[u8; PAYLOAD_LEN]) -> EnvResult<[u8; PAYLOAD_LEN]> {
        // 信封 = `[地址槽 8][载荷 56]`。地址槽那一格就是目录帧的**帧首**（`wire::ADDRESS_AT`），
        // 故"同一帧装进信封"与"裸帧直接 push"两条线形逐字相同（见 `wire` 模块头注）。
        let mut out = [0u8; CALL];
        let _ = put_address(&mut out, self.port.seed());
        out[ADDRESS_LEN..CALL].copy_from_slice(payload);
        self.port.push(&out)?;
        let mut buf = [0u8; CALL];
        let rep = self.port.pull(&mut buf, REPLY_TIMEOUT_MS)?;
        let mut back = [0u8; PAYLOAD_LEN];
        back.copy_from_slice(rep.get(ADDRESS_LEN..CALL).ok_or_else(denied)?);
        Ok(back)
    }

    /// 断开：放下回信孔 + 放下入口门闩。目录不记连接状态，故到此为止。
    ///
    /// 两件事各算各的：`shut` 失败**不阻断**入口的释放（旧 `Channel` 那条 `?` 会一起跳过）。
    pub fn disconnect(self) -> EnvResult<()> {
        let closed = self.port.shut();
        let released = self.entry.release();
        closed?;
        released
    }
}
