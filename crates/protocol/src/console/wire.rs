//! console·wire — 控制台协议的报文编解码（纯函数，零依赖）。
//!
//! # 一帧就是 `[op][正文]`，**只有 `Open` 多一格地址**
//!
//! ```text
//! Open      [0]=1 [1..9) 握手孔  [9..17) 认领号          帧长 17
//! Write     [0]=2 [1..9) client  [9..] 字节              帧长 9 + 字节数
//! ReadLine  [0]=3 [1..9) client  [9..] 提示符            帧长 9 + 提示符长度
//! Close     [0]=4 [1..9) client                          帧长 9
//!
//! Ok        [0]=0 [1..9) client                          帧长 9
//! Line      [0]=1 [1..] 整行（不含 `\n`）                 帧长 1 + 行长
//! Eof       [0]=2 正文 = 空                              帧长 1
//! Interrupt [0]=3 同上                                   帧长 1
//! Denied    [0]=4 同上                                   帧长 1
//! NoSuch…   [0]=5 同上                                   帧长 1
//! ```
//!
//! **没有一处是补的**：没有保留区、没有长度字段、没有定长载荷数组，也没有哪一帧带一格
//! 恒为零的"以防万一"。「这条消息多长」就是「这一帧多少字节」，由成帧的人报给 `Port`；
//! 收方按同一份表精确对账（六个状态**逐个**核长度，短一字节、多一字节都是坏帧）。
//!
//! # 地址槽只在 `Open` 上，它装的是**握手孔**
//!
//! `Open` 那 8 字节是本协议唯一一处地址，它不装"回信孔"——回信孔由服务端开、句柄随
//! 握手回执交回（`runtime::core::handshake::grant`）。它装的是**本端为这次握手开的私有
//! 孔**：服务端收下 `Open` 后把回执推进它（`server::State::open`）。
//!
//! 为什么这一格必须存在（实测）：握手回执若推回**请求孔**，就等于推进服务自己的收件箱
//! ——请求孔是一枚单槽信箱，而服务端的请求循环正在上面 `pull`，谁先拿谁得。读数：
//! 服务建完会话紧接着把自己推的回执吸了回来并当成坏请求拒掉，客户端等满上界，
//! **同一次会话照 50% 的概率开不成**。私有孔把这条路变成一问一答。
//!
//! `Duet::encode` 那三行就是它的落点（机制只问"地址写哪儿"，不问"哪些动词带"）。
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

/// 正文里最大的一段：一次 `Write` 最多带的字节数 = 回复里一行的上界 = **128**。
///
/// 这个数是量出来的，不是拍的：一段字节若超过它，客户端就得把它切成**两条 `Write`**，
/// 而两条之间可以插进**别的写者**的字节——一条 27 字节的报到就这样被劈成
/// `…reconnect` + 别人的一句 + `ed`（实测原样字节：`shell: console reconnect` `root:
/// console restarted\r\n` `ed`），门里那条判据因此常年发飘。故它取**与客户端门面一次
/// flush 的上界同一个数**（`programs/src/bin/user/shell.rs` 的 `Terminal::CAP`）：
/// 一次 flush 永远落成一条消息，谁也不插得进来。
pub const LINE: usize = 128;

/// 一块内存要多大装得下任何一条帧：`op + 一格 usize + LINE`。
///
/// `Open` 那条的**地址槽与认领号**正好占同样两格（`1 + 8 + 8`），也在界内。
pub const CAP: usize = 1 + 2 * WORD + LINE;

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
    /// 开会话，并认领答复。
    ///
    /// 地址槽里留 0（回信孔由服务端开、句柄随答复交回），外加一枚**认领号**：服务端
    /// 把它连同那枚句柄一起推回来，客户端据此从入口孔那一串帧里认出"哪一枚是给我的
    /// 答复"（理由见 `runtime::core::handshake::borrow`）。它当场生成，故对端猜不到
    /// ——"谁在什么时候等哪一枚"这件事不再靠时序。
    Open {
        /// 认领号，原样回。
        nonce: u64,
    },
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

/// `client` 的偏移：紧跟 `op`。仅本模块用。
const CLIENT_AT: usize = 1;
/// `Open` 那一格**认领号**的偏移：它在地址槽之后（其余动词没有地址槽，故那一格紧跟 `op`）。
const NONCE_AT: usize = CLIENT_AT + ADDRESS_LEN;
/// 正文里"变长那一段"的偏移。
const TEXT_AT: usize = CLIENT_AT + WORD;

fn word(m: &[u8], at: usize) -> usize {
    usize::from_le_bytes(m[at..at + WORD].try_into().unwrap_or([0u8; WORD]))
}

impl Query {
    /// 开一次会话。地址是**传输字段**（见模块头），客户端手里没有那枚 token。
    pub const fn open(nonce: u64) -> Query {
        Query::Open { nonce }
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
            // 地址槽 + 认领号：`Open` 是本协议唯一带认领号的帧。
            Query::Open { .. } => 1 + ADDRESS_LEN + WORD,
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
            Query::Open { .. } => (Op::Open, 0, None),
            Query::Write { client, text } => (Op::Write, *client, Some(text)),
            Query::ReadLine { client, prompt } => (Op::ReadLine, *client, Some(prompt)),
            Query::Close { client } => (Op::Close, *client, None),
        };
        m[0] = op as u8;
        if let Some(text) = text {
            let _ = text.encode_into(&mut m, TEXT_AT);
        }
        if let Query::Open { nonce } = self {
            // 认领号在**地址槽之后**（`Open` 没有 client，那两格各归各的）。
            m[NONCE_AT..NONCE_AT + WORD].copy_from_slice(&nonce.to_le_bytes());
        } else {
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
                if body != ADDRESS_LEN + WORD {
                    return Err(ProtocolError::Short);
                }
                Ok(Query::Open {
                    nonce: u64::from_le_bytes(
                        m[NONCE_AT..NONCE_AT + WORD].try_into().unwrap_or([0u8; WORD]),
                    ),
                })
            }
            x if x == Op::Write as u8 => {
                // 空载荷不是"短一字节"而是**这个动词不该有这一格**：`Query::write` 本地就
                // 拒空（分片是客户端的活），解码侧必须同一条表，否则线上能造出本地造不出的帧。
                if body <= WORD {
                    return Err(ProtocolError::BadLen);
                }
                let text = Text::new(&m[TEXT_AT..]).ok_or(ProtocolError::BadLen)?;
                Ok(Query::Write {
                    client: word(m, CLIENT_AT),
                    text,
                })
            }
            x if x == Op::ReadLine as u8 => {
                // 提示符可以为空（`Query::readline` 允许），故只核"client 那一格在不在"。
                if body < WORD {
                    return Err(ProtocolError::Short);
                }
                let prompt = Text::new(&m[TEXT_AT..]).ok_or(ProtocolError::BadLen)?;
                Ok(Query::ReadLine {
                    client: word(m, CLIENT_AT),
                    prompt,
                })
            }
            x if x == Op::Close as u8 => {
                if body != WORD {
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
                let _ = text.encode_into(&mut m, 1);
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
            Reply::Line { text } => 1 + text.as_bytes().len(),
            _ => 1,
        }
    }

    /// 解码。与请求同一张表，按首字节读。
    pub fn decode(m: &[u8]) -> Result<Reply, ProtocolError> {
        let status = *m.first().ok_or(ProtocolError::Short)?;
        let body = m.len() - 1;
        match status {
            STATUS_OK => {
                if body != WORD {
                    return Err(ProtocolError::Short);
                }
                Ok(Reply::Ok {
                    client: word(m, CLIENT_AT),
                })
            }
            STATUS_LINE => {
                // 变长那一支：长度由 `Text` 自己核（> `LINE` ⇒ `BadLen`）。
                let text = Text::new(&m[1..]).ok_or(ProtocolError::BadLen)?;
                Ok(Reply::Line { text })
            }
            // 四条**无正文**的状态：多一个字节就是坏帧（旧版只核了长度，不核这四个——
            // 头注那句"短一字节、多一字节都是坏帧"因此曾经是句空话）。
            STATUS_EOF if body == 0 => Ok(Reply::Eof),
            STATUS_INTERRUPT if body == 0 => Ok(Reply::Interrupt),
            STATUS_DENIED if body == 0 => Ok(Reply::Denied),
            STATUS_NO_CLIENT if body == 0 => Ok(Reply::NoSuchClient),
            STATUS_EOF | STATUS_INTERRUPT | STATUS_DENIED | STATUS_NO_CLIENT => {
                Err(ProtocolError::Short)
            }
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
        if matches!(req, Query::Open { .. }) {
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
