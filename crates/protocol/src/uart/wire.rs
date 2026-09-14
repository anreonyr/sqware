//! uart·wire — 串口协议的线格式（纯函数，零依赖）。
//!
//! # 线格式（请求与回执同一张表，按 `op` 判读）
//!
//! ```text
//! [0]      op      u8        1 = Write
//! [1..9)   地址槽  u64 LE    回执孔在**驱动侧**的 token（每条询问带）
//! [9..)    payload 字节      要写的字节（1..=PAYLOAD_MAX）——**写多少字节帧多长**
//! ```
//!
//! 回执一字节：`Ok`（**字节已进设备**）或 `Denied`（坏报文/超长）。
//!
//! # 两条口径
//!
//! - **只按 `op` 判读字段**，不用哨兵值——与 dispatch / console 同一条规矩。
//! - **回信地址每条询问带**：驱动因此**不需要会话表**，它只认识"这条回执寄到哪"。
//!   这是"驱动只有一台设备、一个客户端"的形状，不是通用做法。
//!
//! # 分片在客户端
//!
//! 一条报文装 `PAYLOAD_MAX` 字节；更长的写由 [`client::Uart::write`] 分片，每片一次往返。
//! 分片不在这里，因为"要不要合并、合并多大"是调用方的节奏（它知道自己一次有多少字节）。

use alloc::vec::Vec;

use env::{EnvError, EnvResult, PieToken, make_err};
use runtime::core::port::{ADDRESS_LEN, Duet, put_address};

/// 本协议的负码：无权 / 协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个；两端共用一份，不各写一遍）。
///
/// **不是设备名**：设备名是设备树给的 `serial@10000000`，用于登记中断线。
pub const SERVICE: &str = "uart";

/// 一条写询问最多带的字节数。
///
/// 为什么是 256：`prog-console` 侧一条输出（一整行 + 重绘）绝大多数落在这一条里，
/// 于是"一行一次跨域"；默认任务栈 16 KiB，收发的报文缓冲各 280 B 不成负担。
pub const PAYLOAD_MAX: usize = 256;

/// 一条帧的固定头：`op + 地址槽`。
pub const HEAD: usize = 1 + ADDRESS_LEN;

/// 一块内存要多大装得下任何一条帧：头 + 最大载荷。
pub const CAP: usize = HEAD + PAYLOAD_MAX;

/// 回执长度：一字节。
pub const ACK_LEN: usize = 1;

/// **投递孔**一次投递的字节数（驱动 → console）。
///
/// 它同时是两个端点的同一个数：驱动一拍从 FIFO 排空多少，console 的输入线程就备多大
/// 的缓冲。**不是本协议的报文长度**——投递孔是**配给**下来的一条孔，不是协议面
/// （上一代叫 `DELIVER_MTU`，`mtu` 制度已废）。
pub const DELIVER: usize = 64;

const OP_AT: usize = 0;
const PAYLOAD_AT: usize = HEAD;

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

/// 一条写询问。
///
/// **回信地址不在询问里**：它是**传输字段**（地址槽 `[1..9)`），由收方用
/// [`address_of`](runtime::core::port::address_of) 从帧里抠、由 [`Duet::encode`]
/// 按本端回信孔填——客户端手里没有那枚 token，故这个类型里也就没有那一格。
///
/// 字节**自有**：`Port::call` 是同步的，但 `Duet::Req` 那一格没有生命周期参数，
/// 询问值借不了调用方那块临时缓冲（`alloc` 可用，一次写一趟本来就有一条跨域往返）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Query {
    bytes: Vec<u8>,
}

impl Query {
    /// 造一条写询问。`bytes` 为空或超过 [`PAYLOAD_MAX`] → `None`（分片是客户端的活）。
    pub fn write(bytes: &[u8]) -> Option<Query> {
        if bytes.is_empty() || bytes.len() > PAYLOAD_MAX {
            return None;
        }
        Some(Query {
            bytes: bytes.to_vec(),
        })
    }

    /// 要写的字节。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 这一帧多少字节：头 + 载荷。**帧长就是长度**，故帧里没有 `len` 字段。
    pub fn len(&self) -> usize {
        PAYLOAD_AT + self.bytes.len()
    }

    /// 编码（**不写地址槽**：留给 [`Duet::encode`]；尾部不补零）。
    pub fn encode(&self) -> ([u8; CAP], usize) {
        let mut m = [0u8; CAP];
        m[OP_AT] = OP_WRITE;
        let n = self.len();
        m[PAYLOAD_AT..n].copy_from_slice(&self.bytes);
        (m, n)
    }

    /// 解码（**零拷贝**）：验动词、验长度区间（长度 = 帧长 − 头）。
    ///
    /// 服务侧用它；未知动词、空载荷、超长一律 `None`——**不 panic**。调用方要回绝的话，
    /// 回信地址仍可用 [`address_of`](runtime::core::port::address_of) 单独抠出来
    /// （坏报文也欠对方一个答复）。
    pub fn view(msg: &[u8]) -> Option<&[u8]> {
        if msg.first() != Some(&OP_WRITE) {
            return None;
        }
        let payload = msg.get(PAYLOAD_AT..)?;
        if payload.is_empty() || payload.len() > PAYLOAD_MAX {
            return None;
        }
        Some(payload)
    }
}

/// 报文对：一条写询问、一字节回执。地址槽在 `[1..9)`，**每条询问都带**。

impl Duet for Query {
    type Req = Query;
    type Rep = Status;
    type Wire = [u8; CAP];

    const CAP: usize = CAP;

    fn wire() -> [u8; CAP] {
        [0u8; CAP]
    }

    /// 回复容器与请求容器分开（理由见 [`Duet::Reply`]）。
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
        put_address(out, at);
        n
    }

    fn decode(buf: &[u8]) -> EnvResult<Status> {
        Status::from_byte(*buf.first().ok_or_else(denied)?).ok_or_else(denied)
    }
}
