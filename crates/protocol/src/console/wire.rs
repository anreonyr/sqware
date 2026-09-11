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
//! # 只有一枚客户端孔
//!
//! ```text
//! 请求孔   客户端 push 请求   → 服务 pull      （客户端持 `entry`）
//! 回信孔   服务 push 回复     → 客户端 pull    （Open 时交出对端 token）
//! ```
//!
//! 第一版还开了一枚"数据孔"（客户端往上推整行）——**它从第一天起就是死码**：
//! 客户端从不推、服务从不读（整行的交付走的是回信孔）。删掉它：每次开会话
//! 少一次 `Channel::open`。
//!
//! # 会话 id 从 1 起
//!
//! 0 恒为"无会话"：`Write{client:0}` / `ReadLine{client:0}` / `Close{0}` 一律
//! `NoSuchClient`。这是 **id 值域**约定，不是字段哨兵（与 `PieToken(0)` 同款）。

/// 单消息字节数（与 dispatch 的 `MSG_LEN` 同值：同一代 hole MTU 协商）。
pub const MSG_LEN: usize = 64;

/// 回信孔 token 在消息里的偏移。
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
    /// 编码为线上消息。
    pub fn encode(&self) -> [u8; MSG_LEN] {
        let mut m = [0u8; MSG_LEN];
        let (op, rpeer, client, len) = match self {
            Request::Open { reply } => (Op::Open, *reply, 0, 0),
            Request::Write { client, len, .. } => (Op::Write, 0, *client, *len),
            Request::ReadLine { client, len, .. } => (Op::ReadLine, 0, *client, *len),
            Request::Close { client } => (Op::Close, 0, *client, 0),
        };
        m[0] = op as u8;
        m[REPLY_PEER_AT..REPLY_PEER_AT + 8].copy_from_slice(&rpeer.to_le_bytes());
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
