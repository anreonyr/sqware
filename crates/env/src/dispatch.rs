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
//! [33..41] entry   usize LE  Register/Replace：入口门闩的目录侧 pie token
//! [41..49] 保留    u64 LE  必须为 0
//! [49..57] reply   usize LE  调用方自带的回信 pie 的目录侧 token（0 = 无回复预期）
//! [57..64] 保留（0）
//! ```
//!
//! `Request::encode/decode` **不**碰 `[49..57]`——调用方在 encode 之后、push 之前
//! 自己填 reply token（参考 `REPLY_AT`）。该字段不是请求字段而是「通道字段」，
//! 与 op/name/entry 同列但语义独立。
//!
//! # 回复（64 字节）
//! ```text
//! [0]      status  u8      0=Ok 1=Found 2=Connected 3=NotFound 4=Denied 5=Taken
//! [1..33]  name    [u8;32] Found：这一页的名字 / Resolve 命中的名字
//! [1..9]   entry   usize LE  Connected：目录转授给调用方的入口门闩 token
//! [9..17]  保留    u64 LE  必须为 0（原 owner 字段已删——服务 task id 由调用方
//!                          用 `MailCall::Owned` 从 entry 的 `owner` 求得）
//! ```
//!
//! # 身份不走消息体
//!
//! 目录认定的调用方身份 = 该请求 `[49..57]` 那枚回信 pie 的 `vestor`——内核在
//! `Accord` 时赋值，消息体伪造不了。没带有效回信 pie 即「无身份」：Register 不看
//! 身份，Unregister/Replace/Connect 一律拒绝。
//!
//! 反方向（调用方认服务）走 `MailCall::Owned`：入口门闩的 `owner` 是**开辟者**
//! （服务自己 `UnsealHole` 出来的），目录转授也改不掉它。**因此服务必须自开入口
//! hole**——若由他人代开，调用方会把回信 hole 授给代开者。
//!
//! # Enumerate 分页
//!
//! 回复只有 64 字节，装不下列表，故按名字**排序**分页：`after` 之后的第一条；
//! `after = None` 从头开始；返 `NotFound` 即到头。

use crate::wire::PieToken;

/// 名字类型与上限的单一真相在 [`crate::wire`]（目录协议与域名字共用）。
pub use crate::wire::{NAME_LEN, Name, NameError};

/// hole 单消息字节数（与内核 `HOLE_MTU_MAX` 协商——dispatch 协议定 64B）。
pub const MSG_LEN: usize = 64;

/// per-caller reply 通道 token 在 wire 中的偏移（`[49..57]`）。
/// 调用方在 `Request::encode()` 之后用此偏移写入 reply pie 的目录侧 token。
pub const REPLY_AT: usize = 49;

const OP_AT: usize = 0;
const NAME_AT: usize = OP_AT + 1;
const ENTRY_AT: usize = NAME_AT + NAME_LEN;

/// `[41..49]` 仍保留（必须为 0）；`[49..57]` 是 reply 通道，decode 不检；`[57..64]` 保留。
const RESERVED_AT: usize = ENTRY_AT + 8;
const RESERVED_END_AT: usize = REPLY_AT + 8;

const REPLY_STATUS_AT: usize = 0;
const REPLY_PAYLOAD_AT: usize = REPLY_STATUS_AT + 1;

const STATUS_OK: u8 = 0;
const STATUS_FOUND: u8 = 1;
const STATUS_CONNECTED: u8 = 2;
const STATUS_NOT_FOUND: u8 = 3;
const STATUS_DENIED: u8 = 4;
const STATUS_TAKEN: u8 = 5;

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
    /// 服务 task id 由调用方用 `MailCall::Owned` 从这枚门闩的 `owner` 求得。
    Connected { entry: PieToken },
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
        // [41..49] 与 [57..64] 必须为 0；[49..57] 是 reply 通道、decode 不检。
        if m[RESERVED_AT..REPLY_AT].iter().any(|&b| b != 0)
            || m[RESERVED_END_AT..].iter().any(|&b| b != 0)
        {
            return Err(ProtocolError::Reserved);
        }
        let entry = PieToken(usize::from_le_bytes(
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
            Reply::Connected { entry } => {
                m[REPLY_STATUS_AT] = STATUS_CONNECTED;
                m[REPLY_PAYLOAD_AT..REPLY_PAYLOAD_AT + 8]
                    .copy_from_slice(&entry.get().to_le_bytes());
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
                let entry = PieToken(usize::from_le_bytes(
                    m[REPLY_PAYLOAD_AT..REPLY_PAYLOAD_AT + 8]
                        .try_into()
                        .unwrap_or([0u8; 8]),
                ));
                Ok(Reply::Connected { entry })
            }
            _ => Err(ProtocolError::BadOp),
        }
    }
}
