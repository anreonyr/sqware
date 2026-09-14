//! uart·wire — 串口协议的线格式（纯函数，零依赖）。
//!
//! # 线格式（请求与回执同一张表，按 `op` 判读）
//!
//! ```text
//! [0]           op      u8        1 = Write
//! [1..8)        保留    7 字节    必须为 0
//! [8..16)       reply   u64 LE    回执孔在**驱动侧**的 token（每条请求带）
//! [16..24)      len     u64 LE    有效字节数（1..=PAYLOAD_MAX）
//! [24..24+256)  payload [u8; 256] 要写的字节（长度以 `len` 为准）
//! ```
//!
//! 回执一字节：`Ok`（**字节已进设备**）或 `Denied`（坏报文/超长）。
//!
//! # 两条口径
//!
//! - **只按 `op` 判读字段**，不用哨兵值——与 dispatch / console 同一条规矩。
//! - **回信地址每条请求带**：驱动因此**不需要会话表**，它只认识"这条回执寄到哪"。
//!   这是"驱动只有一台设备、一个客户端"的形状，不是通用做法。
//!
//! # 分片在客户端
//!
//! 一条报文装 `PAYLOAD_MAX` 字节；更长的写由 [`client::Uart::write`] 分片，每片一次往返。
//! 分片不在这里，因为"要不要合并、合并多大"是调用方的节奏（它知道自己一次有多少字节）。

use env::{EnvError, EnvResult, PieToken, make_err};
use runtime::core::port::Duet;

/// 本协议的负码：无权 / 协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个；两端共用一份，不各写一遍）。
///
/// **不是设备名**：设备名是设备树给的 `serial@10000000`，用于登记中断线。
pub const SERVICE: &str = "uart";

/// 一条写请求最多带的字节数。
///
/// 为什么是 256：`prog-console` 侧一条输出（一整行 + 重绘）绝大多数落在这一条里，
/// 于是"一行一次跨域"；默认任务栈 16 KiB，收发的报文缓冲各 280 B 不成负担。
pub const PAYLOAD_MAX: usize = 256;

/// 请求报文长度 = 头 24 + 载荷 256（`mtu` 取它就是"最大那条报文"）。
pub const MSG_LEN: usize = 24 + PAYLOAD_MAX;

/// 回执长度：一字节。
pub const ACK_LEN: usize = 1;

/// **投递孔**的单消息上限（驱动 → console）。
///
/// 与驱动一拍排空的块同值：一次投递 = 一拍排空的字节。它不是本协议的报文长度——
/// 投递孔是**配给**下来的一条孔，不是协议面。
pub const DELIVER_MTU: usize = 64;

const OP_AT: usize = 0;
const RESERVED_AT: usize = 1;
const REPLY_AT: usize = 8;
const LEN_AT: usize = 16;
const PAYLOAD_AT: usize = 24;

const OP_WRITE: u8 = 1;

/// 回执状态码。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// 字节已进设备（驱动侧 `THRE` 轮询完成才回它）。
    Ok = 0,
    /// 坏报文 / 超长 / 回执地址非法。
    Denied = 1,
}

impl Status {
    pub fn byte(self) -> u8 {
        self as u8
    }

    pub fn from_byte(b: u8) -> Option<Status> {
        match b {
            0 => Some(Status::Ok),
            1 => Some(Status::Denied),
            _ => None,
        }
    }
}

/// 一条写请求。
///
/// **回信地址不在请求里**：它是**通道字段**（`[8..16]`，即 `REPLY_AT`），由收方从
/// 报文里读（[`view`](Request::view) / [`reply_of`]）、由 [`Duet::encode`] 按本端回信孔
/// 填入——客户端手里没有那枚 token，故这个类型里也就没有那一格。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Request {
    /// 有效字节数（`1..=PAYLOAD_MAX`）。
    pub len: usize,
    pub payload: [u8; PAYLOAD_MAX],
}

impl Request {
    /// 造一条写请求。`bytes` 为空或超过 [`PAYLOAD_MAX`] → `None`（分片是客户端的活）。
    pub fn write(bytes: &[u8]) -> Option<Request> {
        if bytes.is_empty() || bytes.len() > PAYLOAD_MAX {
            return None;
        }
        let mut payload = [0u8; PAYLOAD_MAX];
        payload[..bytes.len()].copy_from_slice(bytes);
        Some(Request {
            len: bytes.len(),
            payload,
        })
    }

    /// 编码为线上消息（**不写通道字段**：`[8..16]` 留给 [`Duet::encode`]）。
    pub fn encode(&self) -> [u8; MSG_LEN] {
        let mut m = [0u8; MSG_LEN];
        m[OP_AT] = OP_WRITE;
        m[LEN_AT..LEN_AT + 8].copy_from_slice(&self.len.to_le_bytes());
        m[PAYLOAD_AT..].copy_from_slice(&self.payload);
        m
    }

    /// 解码（**零拷贝**）：验动词、验保留区、验长度，返"要写的字节 + 回执地址"。
    ///
    /// 服务侧用它；未知动词、保留区非零、长度越界一律 `None`——**不 panic**。
    /// 调用方要回绝的话，回执地址仍可用 [`reply_of`] 单独捞出来（坏报文也欠对方一个答复）。
    pub fn view(msg: &[u8]) -> Option<(&[u8], PieToken)> {
        if msg.len() < MSG_LEN || msg[OP_AT] != OP_WRITE {
            return None;
        }
        if msg[RESERVED_AT..REPLY_AT].iter().any(|&b| b != 0) {
            return None;
        }
        let reply = reply_of(msg)?;
        let len = usize::from_le_bytes(msg[LEN_AT..LEN_AT + 8].try_into().ok()?);
        if len == 0 || len > PAYLOAD_MAX {
            return None;
        }
        Some((&msg[PAYLOAD_AT..PAYLOAD_AT + len], reply))
    }
}

/// 从任意报文里捞出回执地址（坏报文也要能答一句 `Denied`）。
///
/// `0` 是本仓的"无句柄"哨兵，故它不算地址。
pub fn reply_of(msg: &[u8]) -> Option<PieToken> {
    if msg.len() < REPLY_AT + 8 {
        return None;
    }
    let token = PieToken::new(usize::from_le_bytes(
        msg[REPLY_AT..REPLY_AT + 8].try_into().ok()?,
    ));
    if token.get() == 0 { None } else { Some(token) }
}

/// 报文对：一条写请求、一字节回执。回信地址写在 `[8..16]`（`REPLY_AT`）。
impl Duet for Request {
    type Req = Request;
    type Rep = Status;
    type Wire = [u8; MSG_LEN];

    const REQ: usize = MSG_LEN;
    const REP: usize = ACK_LEN;

    fn wire() -> [u8; MSG_LEN] {
        [0u8; MSG_LEN]
    }

    fn encode(req: &Request, at: PieToken, out: &mut [u8]) {
        out[..MSG_LEN].copy_from_slice(&req.encode());
        out[REPLY_AT..REPLY_AT + 8].copy_from_slice(&at.get().to_le_bytes());
    }

    fn decode(buf: &[u8]) -> EnvResult<Status> {
        Status::from_byte(*buf.first().ok_or_else(denied)?).ok_or_else(denied)
    }
}
