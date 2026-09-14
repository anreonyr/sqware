//! console·wire — 控制台协议的消息编解码（纯函数，零依赖）。
//!
//! # 字段布局（请求与回复**同一张表**，按 `op` 读）
//!
//! ```text
//! [0]      op      u8
//! [1..8]   保留    7 字节   必须为 0
//! [8..16]  reply   usize LE  Open：调用方**回信孔**在服务侧的 token（服务往它写）
//! [24..32] client  usize LE  Open 回复 / Write / ReadLine / Close：会话 id
//! [32..40] len     u64 LE    Write：有效字节数；Line 回复：行长
//! [40..64] payload [u8;24]   Write：字节；Line 回复：整行（不含 `\n`）
//! ```
//!
//! **只按 `op` 判读字段，不用哨兵值**——dispatch 的 `Request::decode` 就是这么写的
//! （按动词检查保留区）。故"非 Open 的 reply 必须是 0"是一条真检查，
//! 而不是"必须等于某个魔数"。
//!
//! `[8..16]` 是**通道字段**（与 dispatch 的 `[49..57]`、uart 的 `[8..16]` 同一条规矩）：
//! 它由收方从报文里读、由 [`Duet::encode`] 以**本端回信孔在对端表里的句柄**填入——
//! 客户端造请求时给不出也不该给，故 `Request::Open` 的 `reply` 只在 `decode` 之后有意义
//! （值上写 [`PieToken::new(0)`] 的那条路正是 `Request::open`）。
//!
//! # 只有一枚客户端孔
//!
//! ```text
//! 请求孔   客户端 push 请求   → 服务 pull      （客户端持 `entry`）
//! 回信孔   服务 push 回复     → 客户端 pull    （Open 时交出对端 token）
//! ```
//!
//! 第一版还开了一枚"数据孔"（客户端往上推整行）——**它从第一天起就是死码**：
//! 客户端从不推、服务从不读（整行的交付走的是回信孔）。删掉它：每次开会话
//! 少一次自建孔。
//!
//! # 会话 id 从 1 起
//!
//! 0 恒为"无会话"：`Write{client:0}` / `ReadLine{client:0}` / `Close{0}` 一律
//! `NoSuchClient`。这是 **id 值域**约定，不是字段哨兵（与 `PieToken(0)` 同款）。

use env::{EnvError, EnvResult, PieToken, make_err};
use runtime::core::port::Duet;

/// 本协议的负码：无权 / 协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个；客户端与服务端共用一份，不各写一遍）。
///
/// 与 `doom`/`irq` 同形：**名字是协议的一部分**——它是两端认同一台服务的那个词。此前它
/// 是散在程序里的六个字面量（root 的 spawn 表、驱动与 shell 的连接点、服务自己的注册），
/// 谁写错一个字母就得到一次"连不上"，且没有一处可供对账。
pub const SERVICE: &str = "console";

/// 单消息字节数（与 dispatch 的 `MSG_LEN` 同值：同一代协议）。
pub const MSG_LEN: usize = 64;

/// **通道字段**：种在对端表里的那一枚（`To::seed()`）在消息里的偏移。
pub const REPLY_PEER_AT: usize = 8;

/// 会话 id 在消息里的偏移。
pub const CLIENT_AT: usize = 24;

/// 长度字段在消息里的偏移（u64 LE）。
pub const LEN_AT: usize = 32;

/// 载荷起始偏移。
pub const PAYLOAD_AT: usize = 40;

/// 一次 `Write` 能带的字节数（也是回复里一整行的上界）。
pub const PAYLOAD_LEN: usize = MSG_LEN - PAYLOAD_AT;

/// 一整行的字节上界。与 [`PAYLOAD_LEN`] 同值：同一块区两用。
pub const LINE_MAX: usize = PAYLOAD_LEN;

/// 控制台请求动词（线上判别号）。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// 开会话：登记调用方的**回信孔**（对端 token），回执给会话 id。
    Open = 1,
    /// 写：把这批字节交给服务写设备（**同步**：Ok 即"已落屏"）。
    Write = 2,
    /// 读一行（带行编辑）：阻塞到回车 / Ctrl-C / Ctrl-D，结果走回信孔。
    ReadLine = 3,
    /// 关会话。
    Close = 4,
}

/// 回复状态码（线上判别号）。
pub const STATUS_OK: u8 = 0;
pub const STATUS_LINE: u8 = 1;
pub const STATUS_EOF: u8 = 2;
pub const STATUS_INTERRUPT: u8 = 3;
pub const STATUS_DENIED: u8 = 4;
pub const STATUS_NO_CLIENT: u8 = 5;

/// 线格式错误域。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProtocolError {
    /// 未知动词或未知状态码。
    BadOp,
    /// 保留字段非零。
    Reserved,
    /// 该动词下不该出现的字段非零（如 Write 却带了孔 token）。
    UnexpectedField,
    /// 长度越界（`len > PAYLOAD_LEN`）。
    BadLen,
}

/// 控制台请求。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    /// `reply` = 回信孔在服务侧的名字；0 = 非法。
    ///
    /// 客户端造不出来（[`Request::open`] 给的是 0）——它由 `decode` 从**通道字段**
    /// `[8..16]` 读出，由 [`Duet::encode`] 按本端回信孔填入。
    Open { reply: usize },
    /// `payload[..len]` = 要写的字节。
    Write {
        client: usize,
        len: usize,
        payload: [u8; PAYLOAD_LEN],
    },
    /// 读一行。`prompt` = 提示符（重绘时用；空 = 无提示符）。
    ReadLine {
        client: usize,
        len: usize,
        prompt: [u8; PAYLOAD_LEN],
    },
    /// 关会话。
    Close { client: usize },
}

/// 控制台回复。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reply {
    /// Open/Write/Close 成功；Open 时 `client` 是新会话 id。
    Ok { client: usize },
    /// ReadLine：一行（不含 `\n`）。
    Line { len: usize, payload: [u8; LINE_MAX] },
    /// ReadLine：Ctrl-D。
    Eof,
    /// ReadLine：Ctrl-C。
    Interrupt,
    /// 请求非法（保留字段 / 长度越界 / 孔 token 为 0）。
    Denied,
    /// 会话 id 不认识（未开、已关、0）。
    NoSuchClient,
}

fn word(m: &[u8], at: usize) -> usize {
    usize::from_le_bytes(m[at..at + 8].try_into().unwrap_or([0u8; 8]))
}

impl Request {
    /// 开一次会话。回信地址是**通道字段**（见模块头），由 [`Duet::encode`] 填，
    /// 故这里不带参数——客户端手里根本没有那枚 token。
    pub const fn open() -> Request {
        Request::Open { reply: 0 }
    }

    /// 编码为线上消息（**不写通道字段**：`[8..16]` 留给 [`Duet::encode`]）。
    pub fn encode(&self) -> [u8; MSG_LEN] {
        let mut m = [0u8; MSG_LEN];
        let (op, client, len) = match self {
            Request::Open { .. } => (Op::Open, 0, 0),
            Request::Write { client, len, .. } => (Op::Write, *client, *len),
            Request::ReadLine { client, len, .. } => (Op::ReadLine, *client, *len),
            Request::Close { client } => (Op::Close, *client, 0),
        };
        m[0] = op as u8;
        m[CLIENT_AT..CLIENT_AT + 8].copy_from_slice(&client.to_le_bytes());
        m[LEN_AT..LEN_AT + 8].copy_from_slice(&(len as u64).to_le_bytes());
        match self {
            Request::Write { len, payload, .. } => {
                let n = if *len > PAYLOAD_LEN {
                    PAYLOAD_LEN
                } else {
                    *len
                };
                m[PAYLOAD_AT..PAYLOAD_AT + n].copy_from_slice(&payload[..n]);
            }
            Request::ReadLine { len, prompt, .. } => {
                let n = if *len > PAYLOAD_LEN {
                    PAYLOAD_LEN
                } else {
                    *len
                };
                m[PAYLOAD_AT..PAYLOAD_AT + n].copy_from_slice(&prompt[..n]);
            }
            _ => {}
        }
        m
    }

    /// 解码。**按 op 判读字段**：每个动词检查"它不该有的字段必须是 0"。
    pub fn decode(m: &[u8]) -> Result<Request, ProtocolError> {
        if m.len() < MSG_LEN {
            return Err(ProtocolError::Reserved);
        }
        if m[1..REPLY_PEER_AT].iter().any(|&b| b != 0)
            || m[LEN_AT + 8..PAYLOAD_AT].iter().any(|&b| b != 0)
        {
            return Err(ProtocolError::Reserved);
        }
        let op = m[0];
        let rpeer = word(m, REPLY_PEER_AT);
        let client = word(m, CLIENT_AT);
        let len = u64::from_le_bytes(m[LEN_AT..LEN_AT + 8].try_into().unwrap_or([0u8; 8])) as usize;
        let payload_zero = m[PAYLOAD_AT..].iter().all(|&b| b == 0);

        match op {
            x if x == Op::Open as u8 => {
                if rpeer == 0 {
                    return Err(ProtocolError::UnexpectedField);
                }
                if client != 0 || len != 0 || !payload_zero {
                    return Err(ProtocolError::Reserved);
                }
                Ok(Request::Open { reply: rpeer })
            }
            x if x == Op::Write as u8 => {
                if rpeer != 0 {
                    return Err(ProtocolError::UnexpectedField);
                }
                if client == 0 {
                    return Err(ProtocolError::UnexpectedField);
                }
                if len > PAYLOAD_LEN {
                    return Err(ProtocolError::BadLen);
                }
                let mut payload = [0u8; PAYLOAD_LEN];
                payload.copy_from_slice(&m[PAYLOAD_AT..PAYLOAD_AT + PAYLOAD_LEN]);
                Ok(Request::Write {
                    client,
                    len,
                    payload,
                })
            }
            x if x == Op::ReadLine as u8 => {
                if rpeer != 0 {
                    return Err(ProtocolError::UnexpectedField);
                }
                if client == 0 {
                    return Err(ProtocolError::UnexpectedField);
                }
                if len > PAYLOAD_LEN {
                    return Err(ProtocolError::BadLen);
                }
                let mut prompt = [0u8; PAYLOAD_LEN];
                prompt.copy_from_slice(&m[PAYLOAD_AT..PAYLOAD_AT + PAYLOAD_LEN]);
                Ok(Request::ReadLine {
                    client,
                    len,
                    prompt,
                })
            }
            x if x == Op::Close as u8 => {
                if rpeer != 0 {
                    return Err(ProtocolError::UnexpectedField);
                }
                if client == 0 {
                    return Err(ProtocolError::UnexpectedField);
                }
                if len != 0 || !payload_zero {
                    return Err(ProtocolError::Reserved);
                }
                Ok(Request::Close { client })
            }
            _ => Err(ProtocolError::BadOp),
        }
    }
}

impl Reply {
    pub fn encode(&self) -> [u8; MSG_LEN] {
        let mut m = [0u8; MSG_LEN];
        match self {
            Reply::Ok { client } => {
                m[0] = STATUS_OK;
                m[CLIENT_AT..CLIENT_AT + 8].copy_from_slice(&client.to_le_bytes());
            }
            Reply::Line { len, payload } => {
                m[0] = STATUS_LINE;
                let n = if *len > LINE_MAX { LINE_MAX } else { *len };
                m[LEN_AT..LEN_AT + 8].copy_from_slice(&(n as u64).to_le_bytes());
                m[PAYLOAD_AT..PAYLOAD_AT + LINE_MAX].copy_from_slice(payload);
            }
            Reply::Eof => m[0] = STATUS_EOF,
            Reply::Interrupt => m[0] = STATUS_INTERRUPT,
            Reply::Denied => m[0] = STATUS_DENIED,
            Reply::NoSuchClient => m[0] = STATUS_NO_CLIENT,
        }
        m
    }

    pub fn decode(m: &[u8; MSG_LEN]) -> Result<Reply, ProtocolError> {
        match m[0] {
            STATUS_OK => Ok(Reply::Ok {
                client: word(m, CLIENT_AT),
            }),
            STATUS_LINE => {
                let len = u64::from_le_bytes(m[LEN_AT..LEN_AT + 8].try_into().unwrap_or([0u8; 8]))
                    as usize;
                if len > LINE_MAX {
                    return Err(ProtocolError::BadLen);
                }
                let mut payload = [0u8; LINE_MAX];
                payload.copy_from_slice(&m[PAYLOAD_AT..PAYLOAD_AT + LINE_MAX]);
                Ok(Reply::Line { len, payload })
            }
            STATUS_EOF => Ok(Reply::Eof),
            STATUS_INTERRUPT => Ok(Reply::Interrupt),
            STATUS_DENIED => Ok(Reply::Denied),
            STATUS_NO_CLIENT => Ok(Reply::NoSuchClient),
            _ => Err(ProtocolError::BadOp),
        }
    }
}

/// 报文对：一条控制台请求、一条控制台回复。回信地址写在 `[8..16]`（`REPLY_PEER_AT`）。
///
/// **只有 `Open` 那一条带地址**：本协议是"开一次、长期用"——服务在 `Open` 时把回信孔
/// 记进会话表（`server::Slot { reply }`），此后每条请求都照表推回复。其余动词那一格
/// 必须是 0，`decode` 里"非 Open 的 reply 必须为 0"那条因此是真检查。
impl Duet for Request {
    type Req = Request;
    type Rep = Reply;
    type Wire = [u8; MSG_LEN];

    const REQ: usize = MSG_LEN;
    const REP: usize = MSG_LEN;

    fn wire() -> [u8; MSG_LEN] {
        [0u8; MSG_LEN]
    }

    fn encode(req: &Request, at: PieToken, out: &mut [u8]) {
        out[..MSG_LEN].copy_from_slice(&req.encode());
        if matches!(req, Request::Open { .. }) {
            out[REPLY_PEER_AT..REPLY_PEER_AT + 8].copy_from_slice(&at.get().to_le_bytes());
        }
    }

    fn decode(buf: &[u8]) -> EnvResult<Reply> {
        let m: &[u8; MSG_LEN] = buf.try_into().map_err(|_| denied())?;
        Reply::decode(m).map_err(|_| denied())
    }
}
