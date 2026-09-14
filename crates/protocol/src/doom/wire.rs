//! doom·wire — 他杀协议的线格式（纯函数，零依赖）。
//!
//! # 线格式
//!
//! ```text
//! Kill：                                      Quit（无回执）：
//! [0]      op      u8        1                [0] op u8   2
//! [1..33)  target  [u8; 32]  `env::Name`
//! [33..41) ack     usize LE  回信孔在**服务侧**的 token
//! ```
//!
//! 名字的账在**目录**里，不在本协议里：目标只按名字点，服务自己去目录解出
//! "这个名字的活实例是谁"。
//!
//! # 回执四值
//!
//! 一字节，**不把内核分好的失败域压成一档**（`Ok`/`Dead`/`Denied`/`Slow`）——
//! 请求方对前两者的反应完全不同（"它已经没了" vs "不许你动它"）。
//!
//! 回信通道**由调用方自带**（报文里那个 `ack`），与目录协议、控制台协议同一条规矩：
//! 谁发起谁备回信通道。

use env::wire::NAME_LEN;
use env::{EnvError, EnvResult, Name, PieToken, make_err};
use runtime::core::port::Duet;

/// 本协议的负码：无权 / 协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个；两端共用一份，不各写一遍）。
pub const SERVICE: &str = "doom";

/// 请求报文长度：`[op u8][target 32][ack 8]`。
pub const REQ_LEN: usize = 1 + NAME_LEN + 8;

/// 回执长度（一字节）。
pub const ACK_LEN: usize = 1;

/// 动词（号段从 1 起，与 `console`/`dispatch` 同一约定：0 留给"无效"）。
///
/// 只有两个：`Kill`（请求杀一个域，41 字节 + 一字节回执）与 `Quit`（**主人停服**，
/// 1 字节、无回执）。后者不是给客户端的：服务线程在本域、是兄弟线程，**不在血缘里**
/// ——级联收不到它，请求孔又是它自己开的 ⇒ 只能由协议说一句"收场"。
pub const OP_KILL: u8 = 1;
pub const OP_QUIT: u8 = 2;

/// 回执四值。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ack {
    /// 已下令，且已等到**它真的没了**：服务手里那枚指向它的副本被摘掉了（派生链随主人
    /// 断——目标走完了死亡路径）。这条承诺是判据的牙：它成立就不只是"收到请求"。
    Ok,
    /// 没有这个目标 / 目标已回收（对上 Linux 的 `ESRCH`）。
    Dead,
    /// 政策不许：请求者不在 root 允许的名单里（对上 `EPERM`）。
    Denied,
    /// 已下令，但它**还在**（在别的核上跑，等 IPI 自退；或它就是不肯让出）——
    /// **不是失败**，只是这一趟没等到。
    Slow,
}

impl Ack {
    pub const fn byte(self) -> u8 {
        match self {
            Ack::Ok => 0,
            Ack::Dead => 1,
            Ack::Denied => 2,
            Ack::Slow => 3,
        }
    }

    /// 一字节 → 值。**未知取值按 `Denied`**：不认识的回执等于"没成"，
    /// 不假装成功。
    pub const fn from_byte(b: u8) -> Ack {
        match b {
            0 => Ack::Ok,
            1 => Ack::Dead,
            2 => Ack::Denied,
            _ => Ack::Slow,
        }
    }
}

/// 一条杀令：按**名字**点目标。
pub struct Kill {
    pub target: Name,
    /// 回信孔在本协议**服务侧**的 token（服务按此推回执）。
    ///
    /// 这是**通道字段**：`decode` 从报文里读出它，[`Duet::encode`] 按本端回信孔填入；
    /// 客户端造的请求里它是 [`Kill::new`] 给的 0（客户端手里没有那枚 token）。
    pub ack: PieToken,
}

/// **通道字段**：种在对端表里的那一枚（`To::seed()`）在报文里的偏移（见 [`Kill::ack`]）。
const ACK_AT: usize = 1 + NAME_LEN;

impl Kill {
    /// 造一条杀令。回信地址由 [`Duet::encode`] 填，故这里不带参数。
    pub const fn new(target: Name) -> Kill {
        Kill {
            target,
            ack: PieToken::new(0),
        }
    }

    /// 编码为线上消息（**不写通道字段**：`[33..41]` 留给 [`Duet::encode`]）。
    pub fn encode(&self) -> [u8; REQ_LEN] {
        let mut msg = [0u8; REQ_LEN];
        msg[0] = OP_KILL;
        msg[1..ACK_AT].copy_from_slice(self.target.bytes());
        msg
    }

    /// 解码：**长度、动词、名字**三者都要过（非法名不可表达——名字的构造义务在
    /// `Name::from_bytes` 上，这里只是把它接上）。
    pub fn decode(msg: &[u8]) -> Option<Kill> {
        if msg.len() < REQ_LEN || msg[0] != OP_KILL {
            return None;
        }
        let bytes: [u8; NAME_LEN] = msg[1..ACK_AT].try_into().ok()?;
        let target = Name::from_bytes(bytes).ok()?;
        let ack = usize::from_le_bytes(msg[ACK_AT..REQ_LEN].try_into().ok()?);
        Some(Kill {
            target,
            ack: PieToken::new(ack),
        })
    }
}

/// 报文对：一条杀令、一字节回执。回信地址写在 `[33..41]`（`ACK_AT`）。
impl Duet for Kill {
    type Req = Kill;
    type Rep = Ack;
    type Wire = [u8; REQ_LEN];

    const REQ: usize = REQ_LEN;
    const REP: usize = ACK_LEN;

    fn wire() -> [u8; REQ_LEN] {
        [0u8; REQ_LEN]
    }

    fn encode(req: &Kill, at: PieToken, out: &mut [u8]) {
        out[..REQ_LEN].copy_from_slice(&req.encode());
        out[ACK_AT..ACK_AT + 8].copy_from_slice(&at.get().to_le_bytes());
    }

    fn decode(buf: &[u8]) -> EnvResult<Ack> {
        Ok(Ack::from_byte(*buf.first().ok_or_else(denied)?))
    }
}
