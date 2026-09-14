//! console·wire — 控制台协议的报文编解码（纯函数，零依赖）。
//!
//! # 一帧的两段：地址 + 正文
//!
//! ```text
//! [0]       op     u8
//! [1..9)    地址槽  usize LE  **传输字段**：投递孔在服务侧的 token（客户端给不出）
//! [9..]     正文   ……        按 `op` 排；**正文的结束就是这一帧的结束**
//! ```
//!
//! 动词与正文（请求与回复同一张表，按首字节读）：
//!
//! ```text
//! Open      [1]=1  正文 = 空                          帧长 9
//! Write     [1]=2  [9..17) client  [17..] 字节         帧长 17 + 字节数
//! ReadLine  [1]=3  [9..17) client  [17..] 提示符       帧长 17 + 提示符长度
//! Close     [1]=4  [9..17) client                     帧长 17
//!
//! Ok        [1]=0  [9..17) client                     帧长 17
//! Line      [1]=1  [9..] 整行（不含 `\n`）             帧长 9 + 行长
//! Eof       [1]=2  正文 = 空                          帧长 9
//! Interrupt [1]=3  同上                              帧长 9
//! Denied    [1]=4  同上                              帧长 9
//! NoSuch…   [1]=5  同上                              帧长 9
//! ```
//!
//! **没有一处是补的**：正文里不再有保留区、不再有长度字段、不再有定长载荷数组。
//! 「这条消息多长」就是「这一帧多少字节」，由成帧的人报给 `Port`；收方按同一份表
//! 精确对账（短一字节、多一字节都是坏帧）——那才是"这个动词不该有这一格"的真检查，
//! 而它不需要任何字节能为 0 才算数。
//!
//! # 地址槽只在 `Open` 上非零
//!
//! 本协议是"开一次、长期用"：服务在 `Open` 时把回信孔记进会话表（`server::Slot`
//! 的 `reply`），此后每条请求都照表推回复。故只有 `Open` 那一帧带地址，其余动词
//! 的地址槽恒为 0——这是**本协议的**语义，`Duet::encode` 那三行就是它的落点
//! （机制只问"地址写哪儿"，不问"哪些动词带"）。
//!
//! # 会话 id 从 1 起
//!
//! 0 恒为"无会话"：`Write{client:0}` / `ReadLine{client:0}` / `Close{0}` 一律
//! `NoSuchClient`。这是 **id 值域**约定，不是字段哨兵（与 `PieToken(0)` 同款）。

use core::mem::size_of;

use env::{EnvError, EnvResult, PieToken, make_err};
use runtime::core::port::{ADDRESS_LEN, Duet};

/// 地址槽在本协议报文里的偏移（就 `op` 之后那一格）。
///
/// 转出 [`runtime::core::port::ADDRESS_AT`] 是为了让**服务侧读地址**不必知道那个数
/// 从哪儿来：本模块说"槽在这儿"，机制说"槽是这么写的"。
pub use runtime::core::port::ADDRESS_AT;

/// 本协议的负码：协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个；客户端与服务端共用一份，不各写一遍）。
///
/// 与 `doom`/`irq` 同形：**名字是协议的一部分**——它是两端认同一台服务的那个词。此前它
/// 是散在程序里的六个字面量（root 的 spawn 表、驱动与 shell 的连接点、服务自己的注册），
/// 谁写错一个字母就得到一次"连不上"，且没有一处可供对账。
pub const SERVICE: &str = "console";

/// 会话 id 的宽度（`usize` LE）。
pub const WORD: usize = size_of::<usize>();

/// 正文里最大的一段：一次 `Write` 最多带的字节数 = 回复里一行的上界。
pub const LINE: usize = 24;

/// 一块内存要多大装得下任何一条帧：`op + 地址槽 + client + LINE`。
pub const CAP: usize = 1 + ADDRESS_LEN + 2 * WORD + LINE;

/// 正文里那一段字节：容量定长，`bytes[..len]` 有效。
///
/// 那个容量**永不上线**——成帧时只写有效的那一段，故它不是"补零的来源"。它存在的
/// 唯一理由是让这个值能活在帧之外（服务的 `pending` 要跨线程持有一整行）。
///
/// 相等按**有效的那一段**比：容量里没写过的那几格不是一个值的一部分。
#[derive(Clone, Debug)]
pub struct Text {
    bytes: [u8; LINE],
    len: usize,
}

impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Text {}

impl Text {
    /// 由字节造一段。`> LINE` ⇒ `None`（**不截断**：截断会把"写不下"变成"写坏了"）。
    pub fn new(bytes: &[u8]) -> Option<Text> {
        let mut text = Text {
            bytes: [0u8; LINE],
            len: bytes.len(),
        };
        if text.len > LINE {
            return None;
        }
        text.bytes[..text.len].copy_from_slice(bytes);
        Some(text)
    }

    /// 有效的那一段。
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// 写进 `out[at..]` 并返**写了几格**。子切片先造好再拷——越界即 `None`。
    fn encode_into(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let text = self.as_bytes();
        let slot = out.get_mut(at..at + text.len())?;
        slot.copy_from_slice(text);
        Some(text.len())
    }
}

/// 控制台请求。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Query {
    /// 开会话：地址槽里是调用方的回信孔（对端 token）。
    Open,
    /// 写一批字节（**同步**：`Ok` 即"已落屏"）。
    Write { client: usize, text: Text },
    /// 读一行（带行编辑）。`prompt` 供服务侧重绘。
    ReadLine { client: usize, prompt: Text },
    /// 关会话。
    Close { client: usize },
}

/// 控制台回复。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Reply {
    /// Open/Write/Close 成功；Open 时 `client` 是新会话 id。
    Ok { client: usize },
    /// ReadLine：一行（不含 `\n`）。
    Line { text: Text },
    /// ReadLine：Ctrl-D。
    Eof,
    /// ReadLine：Ctrl-C。
    Interrupt,
    /// 请求非法。
    Denied,
    /// 会话 id 不认识（未开、已关、0）。
    NoSuchClient,
}

/// 控制台请求动词（线上判别号）。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// 开会话：登记调用方的回信孔，回执给会话 id。
    Open = 1,
    /// 写：把这批字节交给服务写设备。
    Write = 2,
    /// 读一行（带行编辑）。
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
    /// 长度越界（一段正文超过 `LINE`）。
    BadLen,
    /// 帧长与该动词的表不符（短一字节、多一字节都算）。
    Short,
}

/// 正文里 client 的偏移（`op` 与地址槽之后）。仅本模块用。
const CLIENT_AT: usize = 1 + ADDRESS_LEN;
/// 正文里"变长那一段"的偏移。
const TEXT_AT: usize = CLIENT_AT + WORD;

fn word(m: &[u8], at: usize) -> usize {
    usize::from_le_bytes(m[at..at + WORD].try_into().unwrap_or([0u8; WORD]))
}

impl Query {
    /// 开一次会话。地址是**传输字段**（见模块头），客户端手里没有那枚 token。
    pub const fn open() -> Query {
        Query::Open
    }

    /// 造一条写请求。空或超过 [`LINE`] ⇒ `None`（分片是客户端的活）。
    pub fn write(client: usize, bytes: &[u8]) -> Option<Query> {
        if bytes.is_empty() {
            return None;
        }
        Some(Query::Write {
            client,
            text: Text::new(bytes)?,
        })
    }

    /// 造一条读行请求。提示符可以为空（服务侧无提示符重绘）。
    pub fn readline(client: usize, prompt: &[u8]) -> Option<Query> {
        Some(Query::ReadLine {
            client,
            prompt: Text::new(prompt)?,
        })
    }

    /// 这一帧多少字节。
    pub fn len(&self) -> usize {
        match self {
            Query::Open => 1 + ADDRESS_LEN,
            Query::Write { text, .. } => TEXT_AT + text.as_bytes().len(),
            Query::ReadLine { prompt, .. } => TEXT_AT + prompt.as_bytes().len(),
            Query::Close { .. } => TEXT_AT,
        }
    }

    /// 正文里那一段字节（就地借帧）：`Write` 要写的字节 / `ReadLine` 的提示符。
    ///
    /// 与 [`Query::decode`] 是同一张表的两个读法：那个认动词与会话，这个取变长段。
    pub fn text(m: &[u8]) -> &[u8] {
        m.get(TEXT_AT..).unwrap_or(&[])
    }

    /// 成帧。**地址槽留 0**——由 [`Duet::encode`] 按本端回信孔填。
    pub fn encode(&self) -> ([u8; CAP], usize) {
        let mut m = [0u8; CAP];
        let (op, client, text) = match self {
            Query::Open => (Op::Open, 0, None),
            Query::Write { client, text } => (Op::Write, *client, Some(text)),
            Query::ReadLine { client, prompt } => (Op::ReadLine, *client, Some(prompt)),
            Query::Close { client } => (Op::Close, *client, None),
        };
        m[0] = op as u8;
        if let Some(text) = text {
            let _ = text.encode_into(&mut m, TEXT_AT);
        }
        if !matches!(self, Query::Open) {
            m[CLIENT_AT..CLIENT_AT + WORD].copy_from_slice(&client.to_le_bytes());
        }
        (m, self.len())
    }

    /// 解码。**帧长即那一格**：每个动词只认自己那一种长度。
    pub fn decode(m: &[u8]) -> Result<Query, ProtocolError> {
        let op = *m.first().ok_or(ProtocolError::Short)?;
        let body = m.len() - 1;
        match op {
            x if x == Op::Open as u8 => {
                if body != ADDRESS_LEN {
                    return Err(ProtocolError::Short);
                }
                Ok(Query::Open)
            }
            x if x == Op::Write as u8 => {
                if body < ADDRESS_LEN + WORD {
                    return Err(ProtocolError::Short);
                }
                let text = Text::new(&m[TEXT_AT..]).ok_or(ProtocolError::BadLen)?;
                Ok(Query::Write {
                    client: word(m, CLIENT_AT),
                    text,
                })
            }
            x if x == Op::ReadLine as u8 => {
                if body < ADDRESS_LEN + WORD {
                    return Err(ProtocolError::Short);
                }
                let prompt = Text::new(&m[TEXT_AT..]).ok_or(ProtocolError::BadLen)?;
                Ok(Query::ReadLine {
                    client: word(m, CLIENT_AT),
                    prompt,
                })
            }
            x if x == Op::Close as u8 => {
                if body != ADDRESS_LEN + WORD {
                    return Err(ProtocolError::Short);
                }
                Ok(Query::Close {
                    client: word(m, CLIENT_AT),
                })
            }
            _ => Err(ProtocolError::BadOp),
        }
    }
}

impl Reply {
    /// 成帧。回复**永不带地址**（它走的是对端的回信孔）。
    pub fn encode(&self) -> ([u8; CAP], usize) {
        let mut m = [0u8; CAP];
        let (status, client, text) = match self {
            Reply::Ok { client } => (STATUS_OK, *client, None),
            Reply::Line { text } => (STATUS_LINE, 0, Some(text)),
            Reply::Eof => (STATUS_EOF, 0, None),
            Reply::Interrupt => (STATUS_INTERRUPT, 0, None),
            Reply::Denied => (STATUS_DENIED, 0, None),
            Reply::NoSuchClient => (STATUS_NO_CLIENT, 0, None),
        };
        m[0] = status;
        match text {
            Some(text) => {
                let _ = text.encode_into(&mut m, 1 + ADDRESS_LEN);
            }
            None => {
                if matches!(self, Reply::Ok { .. }) {
                    m[CLIENT_AT..CLIENT_AT + WORD].copy_from_slice(&client.to_le_bytes());
                }
            }
        }
        (m, self.len())
    }

    /// 这一帧多少字节。
    pub fn len(&self) -> usize {
        match self {
            Reply::Ok { .. } => TEXT_AT,
            Reply::Line { text } => 1 + ADDRESS_LEN + text.as_bytes().len(),
            _ => 1 + ADDRESS_LEN,
        }
    }

    /// 解码。与请求同一张表，按首字节读。
    pub fn decode(m: &[u8]) -> Result<Reply, ProtocolError> {
        let status = *m.first().ok_or(ProtocolError::Short)?;
        let body = m.len() - 1;
        match status {
            STATUS_OK => {
                if body != ADDRESS_LEN + WORD {
                    return Err(ProtocolError::Short);
                }
                Ok(Reply::Ok {
                    client: word(m, CLIENT_AT),
                })
            }
            STATUS_LINE => {
                if body < ADDRESS_LEN {
                    return Err(ProtocolError::Short);
                }
                let text = Text::new(&m[1 + ADDRESS_LEN..]).ok_or(ProtocolError::BadLen)?;
                Ok(Reply::Line { text })
            }
            STATUS_EOF => Ok(Reply::Eof),
            STATUS_INTERRUPT => Ok(Reply::Interrupt),
            STATUS_DENIED => Ok(Reply::Denied),
            STATUS_NO_CLIENT => Ok(Reply::NoSuchClient),
            _ => Err(ProtocolError::BadOp),
        }
    }
}

/// 报文对：一条控制台询问、一条控制台应答。尺寸与布局都由本协议说了算——包括
/// **地址槽在 `[1..9)`（`ADDRESS_AT`）**与【地址只在 `Open` 上带】这两件事。
impl Duet for Query {
    type Req = Query;
    type Rep = Reply;
    type Wire = [u8; CAP];

    const CAP: usize = CAP;

    fn wire() -> [u8; CAP] {
        [0u8; CAP]
    }

    /// 回复容器与请求容器分开（理由见 [`Duet::Reply`]）。回复最长为
    /// `1 + ADDRESS_LEN + LINE`（`Line` 那一支），`CAP` 就是它自己的上界。
    type Reply = [u8; CAP];

    fn reply() -> [u8; CAP] {
        [0u8; CAP]
    }

    fn encode(req: &Query, at: PieToken, out: &mut [u8]) -> usize {
        let (frame, n) = req.encode();
        let Some(slot) = out.get_mut(..n) else {
            return 0;
        };
        slot.copy_from_slice(&frame[..n]);
        if matches!(req, Query::Open) {
            if let Some(slot) = out.get_mut(ADDRESS_AT..ADDRESS_AT + ADDRESS_LEN) {
                slot.copy_from_slice(&at.get().to_le_bytes());
            }
        }
        n
    }

    fn decode(buf: &[u8]) -> EnvResult<Reply> {
        Reply::decode(buf).map_err(|_| denied())
    }
}
