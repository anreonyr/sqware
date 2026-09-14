//! dispatch — 服务目录协议（U/S 共享单一真相）。
//!
//! 目录是一个**普通 Service**：内核只负责把「怎么到达目录」交出去（第 1 关已定：
//! 无 `ServiceCall`，入口门闩由父任务 `Accord` 下发），目录自己只有一个 req hole。
//! 本模块定义该 hole 上的消息编解码，不含任何内核机制——没有 Connection、没有
//! Endpoint、没有 Capability。
//!
//! # 询问（变长）
//! ```text
//! [0..8)   地址槽  usize LE  **传输字段**：调用方回信孔在目录侧的 token（每条都带）
//! [8]      op      u8        1=Register 2=Unregister 3=Replace 4=Resolve 5=Enumerate 6=Connect
//! [9..17)  entry   usize LE  Register/Replace：入口门闩的目录侧 token（其余动词必须为 0）
//! [17..)   name    ≤ 31 字节 目标名字——**锚在帧尾**（Enumerate：游标，空 = 从头开始）
//! ```
//!
//! **地址槽排在帧首**（`ADDRESS_AT = 0`），正文紧随其后：这条协议有两张线形——**裸询问**
//! （客户端直接 `push` 进目录的请求孔）与**服务调用载荷**（同一帧装进 `dispatch::Service`
//! 的 64 字节 信封，信封自己也要一格放回信地址）。两者字节必须**逐字相同**，否则同一次
//! 注册换条路就变味；而信封那一格只能落在载荷最前，故帧首就是唯一两边都成立的位置。
//!
//! **名字锚在帧尾**（`entry` 之后）：它的结束由 `entry` 的位置给出、帧长由名字给——名字
//! 多长帧多长，没有终止 NUL、没有填充。定长头 17 字节，去掉名字那一格**正好**是 [`HEAD`]。
//!
//! # 回复（变长）
//! ```text
//! [0]      status  u8      0=Ok 1=Found 2=Connected 3=NotFound 4=Denied 5=Taken
//! [1..)    载荷    Found：名字 / Connected：entry（8 字节）；其余状态无载荷
//! ```
//!
//! 回复**不带地址槽**：它走的是对端那枚回信孔（`push` 到哪儿是 `Port` 的事，不必写在帧里）。
//!
//! `Query::encode/decode` **不碰地址槽**——由 [`Duet::encode`] 写（见模块头那张字段表）。
//! 该字段不是询问字段而是「传输字段」，与 op/name/entry 同列但语义独立。
//!
//! # 身份不走消息体
//!
//! 目录认定的调用方身份 = **内核在 `Push` 时盖章的发送者**（`Pull` 一并交回），
//! 消息体伪造不了。询问里那一格地址槽只是回信地址，且必须确实是该发送者授给目录的
//! 那一枚（否则目录丢弃回复）。
//!
//! # 名字空间由父域预约
//!
//! 目录的绑定表**只能由父域（root）的预约产生**：`Register` 只能**填**已预约的
//! 行（`Unregister` 只能把行里的实例摘空）。未预约的名字 → `NotFound`；预约给
//! 别人的名字 → `Denied`。故「谁能用哪个名字」不是运行时判定，而是表的形状。
//!
//! 反方向（调用方认服务）走 `MailCall::Owned`：入口门闩的 `owner` 是**开辟者**
//! （服务自己 `UnsealHole` 出来的），目录转授也改不掉它。**因此服务必须自开入口
//! hole**——若由他人代开，调用方会把回信 hole 授给代开者。
//!
//! # Enumerate 分页
//!
//! 回复只有 64 字节，装不下列表，故按名字**排序**分页：`after` 之后的第一条；
//! `after = None` 从头开始；返 `NotFound` 即到头。

use env::wire::PieToken;
use env::{EnvError, EnvResult, make_err};
use runtime::core::port::{put_address_at, ADDRESS_LEN, Duet};

/// D1 负码：无权 / 协议错。
pub const E_DENIED: isize = -1;
/// D1 负码：名字无实例 / 未预约。
pub const E_NOT_FOUND: isize = -2;
/// D1 负码：名字已有活实例（内核码 `-1..-7` 之后自取）。
pub const E_TAKEN: isize = -8;

/// 负码 → 错误。三个码各自有名，故不合并成一档。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_DENIED))
}

pub(crate) fn not_found() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_NOT_FOUND))
}

pub(crate) fn taken() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_TAKEN))
}

/// 名字类型与上限的单一真相在 `env::wire`（目录协议与域名字共用）——本 crate 只是
/// 它的使用者，不转口第二遍。
pub use env::wire::{NAME_LEN, Name, NameError};

/// 名字最长能写多少字节（内容上界：定长字段里那一个字节留给终止 NUL）。
pub const TEXT: usize = NAME_LEN - 1;

/// 地址槽在**本协议帧里的偏移**——**帧首**。
///
/// 本协议**覆盖**机制默认的 `[1..9)`：帧首是唯一让"裸询问"与"服务调用载荷"逐字相同的位置
/// （见模块头注）。机制只提供"按偏移读写"的口（[`Duet::ADDRESS_AT`]），偏移由协议说了算。
pub const ADDRESS_AT: usize = 0;

const OP_AT: usize = ADDRESS_AT + ADDRESS_LEN;
const TAIL: usize = 8;
const ENTRY_AT: usize = OP_AT + 1;
/// 定长头宽度（`地址槽 + op + entry`）：名字起于此、止于帧尾；服务侧按它切帧。
pub const HEAD: usize = ENTRY_AT + TAIL;

/// **一块内存要多大装得下任何一条帧**：定长头 + 名字上界。
///
/// 回复比它小（`status + max(名字, entry)`），故容量取两者的大者。这个名字只回答
/// "容器备多大"，**不回答"这一帧多长"**——帧长当场由 [`Query::len`] 给出。
pub const CAP: usize = HEAD + TEXT;

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
    /// 绑定 name → 入口门闩（门闩已委托给目录）。名字须已由父域预约给发起者。
    Register = 1,
    /// 解绑 name（摘实例，**预约行保留**；已发出的权限靠资源死亡自然失效）。
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

/// 目录询问。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Query {
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
    /// 名字无实例（未注册 / 已注销 / 已死）/ 名字未预约 / Enumerate 到头。
    NotFound,
    /// 无权：不是该名字的预约者 / 门闩不可转授 / 回信通道非法。
    Denied,
    /// 名字已有活实例。
    Taken,
}

/// 解码失败域。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProtocolError {
    /// 未知动词或未知状态码。
    BadOp,
    /// 名字非法。
    BadName(NameError),
    /// 该动词下不该有内容的那一格非零（如 Unregister 带了 entry）。
    Reserved,
}

impl Query {
    /// 这一帧多少字节：定长头 + 名字文本长。**这就是"长度即边界"**——没有 `len` 字段，
    /// 也没有补零的尾巴，故这个函数不读 `&self` 的任何运行时状态。
    pub fn len(&self) -> usize {
        HEAD + self.name().map(|n| n.text().len()).unwrap_or(0)
    }

    /// 动词号（线上那一格）。
    fn op(&self) -> u8 {
        match self {
            Query::Register { .. } => Op::Register as u8,
            Query::Unregister { .. } => Op::Unregister as u8,
            Query::Replace { .. } => Op::Replace as u8,
            Query::Resolve { .. } => Op::Resolve as u8,
            Query::Enumerate { .. } => Op::Enumerate as u8,
            Query::Connect { .. } => Op::Connect as u8,
        }
    }

    fn name(&self) -> Option<&Name> {
        match self {
            Query::Register { name, .. }
            | Query::Unregister { name }
            | Query::Replace { name, .. }
            | Query::Resolve { name }
            | Query::Connect { name } => Some(name),
            Query::Enumerate { after } => after.as_ref(),
        }
    }

    fn entry(&self) -> PieToken {
        match self {
            Query::Register { entry, .. } | Query::Replace { entry, .. } => *entry,
            _ => PieToken::new(0),
        }
    }

    /// 把正文写进 `out`——**地址槽那一格跳过**（它归 [`Duet::encode`] 填，见模块头注）。
    ///
    /// 返**本帧的字节数**。
    fn put(&self, out: &mut [u8]) -> usize {
        let n = self.len();
        let Some(m) = out.get_mut(..n) else {
            return 0;
        };
        m[OP_AT] = self.op();
        m[ENTRY_AT..ENTRY_AT + TAIL].copy_from_slice(&self.entry().get().to_le_bytes());
        if let Some(name) = self.name() {
            m[HEAD..n].copy_from_slice(name.text());
        }
        n
    }

    /// 解码：动词、名字、entry 三者都要过。**帧长即那一格**——名字起于定长头、止于
    /// 帧尾，故不再有"终止 NUL + 全零填充"。地址槽那一格不看（它的读者是服务侧）。
    pub fn decode(m: &[u8]) -> Result<Query, ProtocolError> {
        if m.len() < HEAD {
            return Err(ProtocolError::BadName(NameError::Empty));
        }
        let op = m[OP_AT];
        if !(1..=6).contains(&op) {
            return Err(ProtocolError::BadOp);
        }
        let text = &m[HEAD..];
        let entry = PieToken(usize::from_le_bytes(
            m[ENTRY_AT..ENTRY_AT + TAIL].try_into().unwrap_or([0u8; 8]),
        ));
        // 只有 Register/Replace 带 entry；其余动词那一格必须为 0（真检查：帧里它存在）。
        if !matches!(op, x if x == Op::Register as u8 || x == Op::Replace as u8) && entry.get() != 0
        {
            return Err(ProtocolError::Reserved);
        }
        let name = || Name::from_slice(text).map_err(ProtocolError::BadName);
        match op {
            x if x == Op::Register as u8 => Ok(Query::Register {
                name: name()?,
                entry,
            }),
            x if x == Op::Unregister as u8 => Ok(Query::Unregister { name: name()? }),
            x if x == Op::Replace as u8 => Ok(Query::Replace {
                name: name()?,
                entry,
            }),
            x if x == Op::Resolve as u8 => Ok(Query::Resolve { name: name()? }),
            // 游标那一段为空 = 从头开始（空名字本就不是名字，故这一格无歧义）。
            x if x == Op::Enumerate as u8 => Ok(Query::Enumerate {
                after: if text.is_empty() { None } else { Some(name()?) },
            }),
            _ => Ok(Query::Connect { name: name()? }),
        }
    }
}

impl Reply {
    /// 这一帧多少字节。
    pub fn len(&self) -> usize {
        match self {
            Reply::Found { name } => REPLY_PAYLOAD_AT + name.text().len(),
            Reply::Connected { .. } => REPLY_PAYLOAD_AT + TAIL,
            _ => REPLY_PAYLOAD_AT,
        }
    }

    /// 编码（回复**永不带地址**：它走的是对端的回信孔）。
    pub fn encode(&self) -> ([u8; CAP], usize) {
        let mut m = [0u8; CAP];
        let n = self.len();
        m[REPLY_STATUS_AT] = match self {
            Reply::Ok => STATUS_OK,
            Reply::NotFound => STATUS_NOT_FOUND,
            Reply::Denied => STATUS_DENIED,
            Reply::Taken => STATUS_TAKEN,
            Reply::Found { .. } => STATUS_FOUND,
            Reply::Connected { .. } => STATUS_CONNECTED,
        };
        match self {
            Reply::Found { name } => {
                m[REPLY_PAYLOAD_AT..n].copy_from_slice(name.text());
            }
            Reply::Connected { entry } => {
                m[REPLY_PAYLOAD_AT..n].copy_from_slice(&entry.get().to_le_bytes());
            }
            _ => {}
        }
        (m, n)
    }

    /// 解码：与询问同一张表，按首字节读。载荷长度 = 帧长 − 头。
    pub fn decode(m: &[u8]) -> Result<Reply, ProtocolError> {
        let status = *m.first().ok_or(ProtocolError::BadOp)?;
        let payload = m
            .get(REPLY_PAYLOAD_AT..)
            .ok_or(ProtocolError::BadName(NameError::Empty))?;
        match status {
            STATUS_OK => Ok(Reply::Ok),
            STATUS_NOT_FOUND => Ok(Reply::NotFound),
            STATUS_DENIED => Ok(Reply::Denied),
            STATUS_TAKEN => Ok(Reply::Taken),
            STATUS_FOUND => Ok(Reply::Found {
                name: Name::from_slice(payload).map_err(ProtocolError::BadName)?,
            }),
            STATUS_CONNECTED => Ok(Reply::Connected {
                entry: PieToken(usize::from_le_bytes(payload.try_into().unwrap_or([0u8; 8]))),
            }),
            _ => Err(ProtocolError::BadOp),
        }
    }
}

/// 报文对：一条目录询问、一条目录应答。尺寸与布局都由本协议说了算——包括
/// **地址槽在帧首**（本协议每问都带）与**帧长 = 定长头 + 名字文本长**这两件事。
impl Duet for Query {
    type Req = Query;
    type Rep = Reply;
    type Wire = [u8; CAP];

    /// 帧首（[`ADDRESS_AT`]）——与信封那一格**同一个位置**，故两条线形逐字相同。
    const ADDRESS_AT: usize = ADDRESS_AT;
    const CAP: usize = CAP;

    fn wire() -> [u8; CAP] {
        [0u8; CAP]
    }

    /// 回信地址（帧首那一格）+ 正文（[`Query::put`] 跳过那一格往后面写）。帧长即
    /// [`Query::len`]，`out` 的其余位置一个字节都不上线。
    fn encode(req: &Query, at: PieToken, out: &mut [u8]) -> usize {
        let n = req.put(out);
        let _ = put_address_at(out, ADDRESS_AT, at);
        n
    }

    fn decode(buf: &[u8]) -> EnvResult<Reply> {
        Reply::decode(buf).map_err(|_| denied())
    }
}
