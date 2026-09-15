//! doom·wire — 他杀协议的线格式（纯函数，零依赖）。
//!
//! # 线格式
//!
//! ```text
//! Kill：                                      Quit（无回执）：
//! [0]      op      u8        1                [0] op u8   2
//! [1..9)   地址槽  usize LE  回信孔在**服务侧**的 token（每条询问都带）
//! [9..)    target  ≤ 31 字节 `env::Name`——**名字多长帧多长**
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
/// 本协议的负码：无权 / 协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个；两端共用一份，不各写一遍）。
pub const SERVICE: &str = "doom";

/// 名字最长能写多少字节（内容上界：定长字段里那一个字节留给终止 NUL）。
pub const TEXT: usize = NAME_LEN - 1;

/// 回信地址的线形：一枚 `PieToken` 的 8 字节小端。
pub const ADDRESS_LEN: usize = 8;

/// 回信地址在**本协议**帧里的偏移：紧跟 `op`。
pub const ADDRESS_AT: usize = 1;

/// 把回信地址写进本协议帧的那一格。
pub fn put_address(out: &mut [u8], token: PieToken) -> Option<()> {
    let slot = out.get_mut(ADDRESS_AT..ADDRESS_AT + ADDRESS_LEN)?;
    slot.copy_from_slice(&token.get().to_le_bytes());
    Some(())
}

/// 从帧里抠出回信地址——**坏报文也要能抠**（拒一条请求仍欠对方一句）。`None` = 太短 /
/// 全是 0（0 是本仓的"无句柄"）。
pub fn address_of(m: &[u8]) -> Option<PieToken> {
    let slot = m.get(ADDRESS_AT..ADDRESS_AT + ADDRESS_LEN)?;
    let token = PieToken::new(usize::from_le_bytes(slot.try_into().ok()?));
    if token.get() == 0 { None } else { Some(token) }
}

/// 一块内存要多大装得下任何一条帧：`op + 地址槽 + 名字(上界)`。
pub const CAP: usize = 1 + ADDRESS_LEN + TEXT;

/// 回执长度（一字节）。
pub const ACK_LEN: usize = 1;

/// 名字在帧里的偏移。
const NAME_AT: usize = 1 + ADDRESS_LEN;

/// 动词（号段从 1 起，与 `console`/`dispatch` 同一约定：0 留给"无效"）。
///
/// 只有两个：`Kill`（请求杀一个域，名字多长帧多长 + 一字节回执）与 `Quit`（**主人停服**，
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

    /// 解一条回执。空报文 ⇒ `Denied`（`from_byte` 对未知取值也给 `Denied`，故这里只多了
    /// "一个字节都没有"那一档）。
    pub fn decode(buf: &[u8]) -> EnvResult<Ack> {
        Ok(Ack::from_byte(*buf.first().ok_or_else(denied)?))
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

/// 一条询问：按**名字**点目标。
///
/// 值里**没有回信地址**：它是**传输字段**（地址槽），由 `decode` 之后的收方用
/// [`address_of`] 从报文里抠、由 [`Query::encode`] 按本端回信孔填——客户端手里
/// 没有那枚 token。
pub struct Query {
    pub target: Name,
}

impl Query {
    /// 造一条杀令。回信地址由 [`Query::encode`] 填，故这里不带参数。
    pub const fn new(target: Name) -> Query {
        Query { target }
    }

    /// 这一帧多少字节：头 9 + 名字那一段。
    pub fn len(&self) -> usize {
        NAME_AT + self.target.text().len()
    }

    /// 编码：写出整帧，并把**回信地址**填进本协议那一格。尾部不补零——帧长就是那个
    /// 返回值。
    pub fn encode(&self, at: PieToken) -> ([u8; CAP], usize) {
        let mut msg = [0u8; CAP];
        msg[0] = OP_KILL;
        let n = self.len();
        msg[NAME_AT..n].copy_from_slice(self.target.text());
        let _ = put_address(&mut msg, at);
        (msg, n)
    }

    /// 解码：**动词、名字**两者都要过（非法名不可表达——名字的构造义务在
    /// `Name::from_slice` 上，这里只是把它接上）。
    pub fn decode(msg: &[u8]) -> Option<Query> {
        if *msg.first()? != OP_KILL {
            return None;
        }
        let target = Name::from_slice(msg.get(NAME_AT..)?).ok()?;
        Some(Query { target })
    }
}
