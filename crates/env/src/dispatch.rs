//! dispatch — 服务目录协议（U/S 共享单一真相）。
//!
//! 目录是一个**普通 Service**：内核只负责把「怎么到达目录」交出去（第 1 关已定：
//! 无 `ServiceCall`，入口门闩由父任务 `Accord` 下发），目录自己只有一个 req hole。
//! 本模块定义该 hole 上的消息编解码，不含任何内核机制——没有 Connection、没有
//! Endpoint、没有 Capability。
//!
//! # 请求（64 字节）
//! ```text
//! [0]      op      u8      1=Register 2=Unregister 3=Replace 4=Resolve 5=Enumerate 6=Connect
//! [1..33]  name    [u8;32] Register/Unregister/Replace/Resolve/Connect：目标名字
//!                          Enumerate：游标（全 0 = 从头开始）
//! [33..41] entry   u64 LE  Register/Replace：入口门闩的目录侧 pie token
//! [41..49] 保留    u64 LE  必须为 0
//! [49..57] reply   u64 LE  **保留（v1 必须为 0）**：回信通道由内核在 boot 期
//!                          预置（目录 → 主 client 的 hole），不随请求传递。
//!                          per-caller 通道留待多 client（该字段即接入点）。
//! [57..64] 保留（0）
//! ```
//!
//! # 回复（64 字节）
//! ```text
//! [0]      status  u8      0=Ok 1=Found 2=Connected 3=NotFound 4=Denied 5=Taken
//! [1..33]  name    [u8;32] Found：这一页的名字 / Resolve 命中的名字
//! [1..9]   entry   u64 LE  Connected：目录转授给调用方的入口门闩 token
//! [9..17]  owner   u64 LE  Connected：服务的 owner task id（调用方把回信 hole
//!                          Accord 给它）
//! ```
//!
//! # 身份不走消息体
//!
//! 目录认定的调用方身份来自**内核**（boot 期把主 client 的 task id 交给目录），
//! 或（多 client 时）来自调用方委托的回信 pie 的 `vestor`——消息体里的任何字段
//! 都不参与身份判定，故不可伪造。
//!
//! # Enumerate 分页
//!
//! 回复只有 64 字节，装不下列表，故按名字**排序**分页：`after` 之后的第一条；
//! `after = None` 从头开始；返 `NotFound` 即到头。

use crate::wire::{PieToken, TaskId};

/// 名字字段字节数（含终止 NUL）。
pub const NAME_LEN: usize = 32;

/// hole 单消息字节数（与内核 `HOLE_MSG_LEN`、用户 `HOLE_MSG_LEN` 一致）。
pub const MSG_LEN: usize = 64;

const OP_AT: usize = 0;
const NAME_AT: usize = OP_AT + 1;
const ENTRY_AT: usize = NAME_AT + NAME_LEN;
/// [41..64) 全保留（v1 必须为 0；[49..57] 是未来 per-caller 回信通道的位置）。
const RESERVED_AT: usize = ENTRY_AT + 8;

const REPLY_STATUS_AT: usize = 0;
const REPLY_PAYLOAD_AT: usize = REPLY_STATUS_AT + 1;

const STATUS_OK: u8 = 0;
const STATUS_FOUND: u8 = 1;
const STATUS_CONNECTED: u8 = 2;
const STATUS_NOT_FOUND: u8 = 3;
const STATUS_DENIED: u8 = 4;
const STATUS_TAKEN: u8 = 5;

/// 名字校验失败域。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameError {
    /// 空名。
    Empty,
    /// 超出 [`NAME_LEN`] - 1 字节（要留终止 NUL）。
    TooLong,
    /// 含 NUL（会与填充歧义）。
    Nul,
}

/// 服务名：定长 32 字节、尾随 NUL 填充、内容非空且不含 NUL。
///
/// 类型义务：非法名不可表达——拿到 `Name` 即已校验，协议层不再查；比较按整块
/// 定长字节（填充由构造保证规范，故等值即语义等值）。`Hash`/`Ord` 供目录容器
/// 与排序枚举使用。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Name {
    bytes: [u8; NAME_LEN],
}

impl Name {
    /// 由字符串构造（校验失败即拒绝，不截断）。
    pub fn new(s: &str) -> Result<Name, NameError> {
        let b = s.as_bytes();
        if b.is_empty() {
            return Err(NameError::Empty);
        }
        if b.len() >= NAME_LEN {
            return Err(NameError::TooLong);
        }
        if b.contains(&0) {
            return Err(NameError::Nul);
        }
        let mut bytes = [0u8; NAME_LEN];
        bytes[..b.len()].copy_from_slice(b);
        Ok(Name { bytes })
    }

    /// 由线上字节还原（校验填充规范 + 内容合法）。
    fn from_bytes(bytes: [u8; NAME_LEN]) -> Result<Name, NameError> {
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        if len == 0 {
            return Err(NameError::Empty);
        }
        if bytes[len..].iter().any(|&b| b != 0) {
            return Err(NameError::Nul);
        }
        if core::str::from_utf8(&bytes[..len]).is_err() {
            return Err(NameError::Nul);
        }
        Ok(Name { bytes })
    }

    /// 定长字节视图（含填充）。
    pub fn bytes(&self) -> &[u8; NAME_LEN] {
        &self.bytes
    }

    /// 内容长度（终止 NUL 之前）。
    pub fn len(&self) -> usize {
        self.bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 名字文本（构造已保证 UTF-8）。
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len()]).unwrap_or("")
    }
}

/// 目录请求动词（线上判别号）。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// 绑定 name → 入口门闩（门闩已委托给目录）。
    Register = 1,
    /// 解绑 name（只管名字；已发出的权限靠资源死亡自然失效）。
    Unregister = 2,
    /// 换绑：服务重启换了门闩，名字不变。
    Replace = 3,
    /// 按名探测（纯读，不改调用方权限表）。
    Resolve = 4,
    /// 枚举一页（按名排序，游标分页）。
    Enumerate = 5,
    /// 连接：目录把入口门闩转授一份给调用方。
    Connect = 6,
}

/// 目录请求。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    Register {
        name: Name,
        entry: PieToken,
    },
    Unregister {
        name: Name,
    },
    Replace {
        name: Name,
        entry: PieToken,
    },
    Resolve {
        name: Name,
    },
    Enumerate {
        /// 游标：None = 从头开始。
        after: Option<Name>,
    },
    Connect {
        name: Name,
    },
}

/// 目录回复。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reply {
    /// Register / Unregister / Replace 成功。
    Ok,
    /// Enumerate 的一页。
    Found { name: Name },
    /// Connect 成功：目录已把入口门闩放进调用方权限表。
    Connected { entry: PieToken, owner: TaskId },
    /// 无名 / Enumerate 到头。
    NotFound,
    /// 无权 / 门闩不可转授 / 回信通道非法。
    Denied,
    /// 名字已占用。
    Taken,
}

/// 解码失败域。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProtocolError {
    /// 未知动词或未知状态码。
    BadOp,
    /// 名字非法。
    BadName(NameError),
    /// 该动词下保留字段非零。
    Reserved,
}

impl Request {
    pub fn encode(&self) -> [u8; MSG_LEN] {
        let mut m = [0u8; MSG_LEN];
        let (op, name, entry) = match self {
            Request::Register { name, entry } => (Op::Register, Some(name), entry.get()),
            Request::Unregister { name } => (Op::Unregister, Some(name), 0),
            Request::Replace { name, entry } => (Op::Replace, Some(name), entry.get()),
            Request::Resolve { name } => (Op::Resolve, Some(name), 0),
            Request::Enumerate { after } => (Op::Enumerate, after.as_ref(), 0),
            Request::Connect { name } => (Op::Connect, Some(name), 0),
        };
        m[OP_AT] = op as u8;
        if let Some(name) = name {
            m[NAME_AT..NAME_AT + NAME_LEN].copy_from_slice(name.bytes());
        }
        m[ENTRY_AT..ENTRY_AT + 8].copy_from_slice(&entry.to_le_bytes());
        m
    }

    pub fn decode(m: &[u8]) -> Result<Request, ProtocolError> {
        if m.len() < MSG_LEN {
            return Err(ProtocolError::BadName(NameError::Empty));
        }
        let op = m[OP_AT];
        if !(1..=6).contains(&op) {
            return Err(ProtocolError::BadOp);
        }
        if m[RESERVED_AT..].iter().any(|&b| b != 0) {
            return Err(ProtocolError::Reserved);
        }
        let entry = PieToken(u64::from_le_bytes(
            m[ENTRY_AT..ENTRY_AT + 8].try_into().unwrap_or([0u8; 8]),
        ));
        let mut name_bytes = [0u8; NAME_LEN];
        name_bytes.copy_from_slice(&m[NAME_AT..NAME_AT + NAME_LEN]);
        let name = || Name::from_bytes(name_bytes).map_err(ProtocolError::BadName);

        match op {
            x if x == Op::Register as u8 => Ok(Request::Register {
                name: name()?,
                entry,
            }),
            x if x == Op::Unregister as u8 => {
                if entry.get() != 0 {
                    return Err(ProtocolError::Reserved);
                }
                Ok(Request::Unregister { name: name()? })
            }
            x if x == Op::Replace as u8 => Ok(Request::Replace {
                name: name()?,
                entry,
            }),
            x if x == Op::Resolve as u8 => {
                if entry.get() != 0 {
                    return Err(ProtocolError::Reserved);
                }
                Ok(Request::Resolve { name: name()? })
            }
            x if x == Op::Enumerate as u8 => {
                if entry.get() != 0 {
                    return Err(ProtocolError::Reserved);
                }
                // 游标全 0 = 从头开始；否则必须是合法名字。
                let after = if name_bytes.iter().all(|&b| b == 0) {
                    None
                } else {
                    Some(name()?)
                };
                Ok(Request::Enumerate { after })
            }
            _ => {
                if entry.get() != 0 {
                    return Err(ProtocolError::Reserved);
                }
                Ok(Request::Connect { name: name()? })
            }
        }
    }
}

impl Reply {
    pub fn encode(&self) -> [u8; MSG_LEN] {
        let mut m = [0u8; MSG_LEN];
        match self {
            Reply::Ok => m[REPLY_STATUS_AT] = STATUS_OK,
            Reply::NotFound => m[REPLY_STATUS_AT] = STATUS_NOT_FOUND,
            Reply::Denied => m[REPLY_STATUS_AT] = STATUS_DENIED,
            Reply::Taken => m[REPLY_STATUS_AT] = STATUS_TAKEN,
            Reply::Found { name } => {
                m[REPLY_STATUS_AT] = STATUS_FOUND;
                m[REPLY_PAYLOAD_AT..REPLY_PAYLOAD_AT + NAME_LEN].copy_from_slice(name.bytes());
            }
            Reply::Connected { entry, owner } => {
                m[REPLY_STATUS_AT] = STATUS_CONNECTED;
                m[REPLY_PAYLOAD_AT..REPLY_PAYLOAD_AT + 8]
                    .copy_from_slice(&entry.get().to_le_bytes());
                m[REPLY_PAYLOAD_AT + 8..REPLY_PAYLOAD_AT + 16]
                    .copy_from_slice(&(owner.get() as u64).to_le_bytes());
            }
        }
        m
    }

    pub fn decode(m: &[u8; MSG_LEN]) -> Result<Reply, ProtocolError> {
        match m[REPLY_STATUS_AT] {
            STATUS_OK => Ok(Reply::Ok),
            STATUS_NOT_FOUND => Ok(Reply::NotFound),
            STATUS_DENIED => Ok(Reply::Denied),
            STATUS_TAKEN => Ok(Reply::Taken),
            STATUS_FOUND => {
                let mut name_bytes = [0u8; NAME_LEN];
                name_bytes.copy_from_slice(&m[REPLY_PAYLOAD_AT..REPLY_PAYLOAD_AT + NAME_LEN]);
                Ok(Reply::Found {
                    name: Name::from_bytes(name_bytes).map_err(ProtocolError::BadName)?,
                })
            }
            STATUS_CONNECTED => {
                let entry = PieToken(u64::from_le_bytes(
                    m[REPLY_PAYLOAD_AT..REPLY_PAYLOAD_AT + 8]
                        .try_into()
                        .unwrap_or([0u8; 8]),
                ));
                let owner = TaskId(u64::from_le_bytes(
                    m[REPLY_PAYLOAD_AT + 8..REPLY_PAYLOAD_AT + 16]
                        .try_into()
                        .unwrap_or([0u8; 8]),
                ) as usize);
                Ok(Reply::Connected { entry, owner })
            }
            _ => Err(ProtocolError::BadOp),
        }
    }
}
