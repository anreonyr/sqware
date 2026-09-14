//! irq·wire — 中断线协议的线格式（纯函数，零依赖）。
//!
//! # 两个动词、定长 49 字节
//!
//! ```text
//! [0]       op    u8        1 Register / 2 Refer
//! [1..9)    arg   u64 LE    Register：会话门闩在**驱动侧**的 token
//!                            Refer   ：把名字交给谁（who）
//! [9..17)   ack   u64 LE    回信孔在**驱动侧**的 token
//! [17..49)  name  [u8; 32]  设备名（boot 给的设备树节点 basename）
//! ```
//!
//! **线号不在报文里**：线 = 名字的函数（名字 → 设备树节点 → `interrupts` ×
//! `interrupt-parent`），由驱动自己解出来（`docs/driver.md` §12 甲）。客户端因此
//! 既报不出、也抢不走一条不属于它的线——报文里没有可伪造的线号。
//!
//! # 属主只能由 root 写
//!
//! `Refer` 的推者必须是驱动域的 `sire`（= root 的主线程，生它的那个任务）：**谁能用哪个
//! 名字不是运行时判定，而是表里有没有写你的行**——与 `dispatch` 的 `Refer` 同形。
//! 客户端只能 `Register` **自己那一行**。
//!
//! # 回执一字节：成功 或 四种拒绝之一
//!
//! 四值不合并（请求方对它们的反应不同）：名字不认识（`Unknown`）是配错名，没人认领
//! （`Unclaimed`）是 root 还没写属主，**不是你的**（`NotYours`）是权威判定，已经有人占着
//! （`Taken`）是旧实例没收线。回信通道**由调用方自带**（报文里那个 `ack`），与目录协议、
//! 控制台协议、他杀协议同一条规矩：谁发起谁备回信通道。
//!
//! # 投递载荷：u16 线号
//!
//! 驱动投给客户端的是**线号**（[`LINE_LEN`] 字节，`docs/driver.md` §5.3："长度是参数不是
//! 约定"——实测 `ndev = 95`，一个字节够，仍给 u16）。客户端核**来源**（`from` 是内核在
//! `Push` 时盖的章）与长度；线号本身它不认识，也不该认识。

use env::wire::NAME_LEN;
use env::{EnvError, EnvResult, Name, PieToken, TaskId, make_err};
use runtime::core::port::Duet;

/// 本协议的负码：无权 / 协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个）。
pub const SERVICE: &str = "plic";

/// 报文长度：`[op][arg 8][ack 8][name 32]`——**三个动词同一条记录**，故 `mtu` 就是它。
pub const LEN: usize = 1 + 8 + 8 + NAME_LEN;

/// 投递载荷（线号）的字节数。
pub const LINE_LEN: usize = 2;

/// **通道字段**：种在对端表里的那一枚（`To::seed()`）在报文里的偏移（见三个构造器的注）。
const ACK_AT: usize = 9;

const OP_REGISTER: u8 = 1;
const OP_REFER: u8 = 2;
const OP_DELEGATE: u8 = 3;

/// 一条请求：**动词 + 一条定长记录**。
///
/// 线形是统一的（见模块头），但两侧都用枚举说话：驱动不可能把 `who` 当成会话门闩读
/// ——那是解码时就已经分好的事。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    /// 登记：我按名字拿到的这台设备，会话门闩在那儿。
    Register {
        name: Name,
        session: PieToken,
        ack: PieToken,
    },
    /// 写属主：这个名字归 `who`（**只有 root 会发它**）。
    Refer {
        name: Name,
        who: TaskId,
        ack: PieToken,
    },
    /// 委托写权：**这个名字的属主，从此也可以由 `who` 写**（同样只有 root 会发它）。
    ///
    /// 为什么需要它：root 的服务重启落在**它自己域里的监护线程**上（`docs/root.md` §5.3），
    /// 而线程不是域——驱动认的是"推者是不是我的 `sire`"，监护线程答不上这一条。放弃这条
    /// 判据不行（那等于谁都能改属主）；把判据放宽成"同一个域"也不行（驱动拿不到推者的域）。
    /// 于是让 **root 亲口把这份写权委托给它信任的那个线程**：判据从"谁的 id"变成
    /// "root 有没有为这个名字委托过你"——一条可审计、可收回（重启即失效）的授权。
    Delegate {
        name: Name,
        who: TaskId,
        ack: PieToken,
    },
}

impl Request {
    /// 三个构造器都**不收回信地址**：它是**通道字段**（`[9..17]`），由 `decode` 从
    /// 报文里读、由 [`Duet::encode`] 按本端回信孔填——客户端手里没有那枚 token，
    /// 故请求值里它是 0。
    pub const fn register(name: Name, session: PieToken) -> Request {
        Request::Register {
            name,
            session,
            ack: PieToken::new(0),
        }
    }

    pub const fn refer(name: Name, who: TaskId) -> Request {
        Request::Refer {
            name,
            who,
            ack: PieToken::new(0),
        }
    }

    pub const fn delegate(name: Name, who: TaskId) -> Request {
        Request::Delegate {
            name,
            who,
            ack: PieToken::new(0),
        }
    }

    pub fn name(&self) -> Name {
        match self {
            Request::Register { name, .. }
            | Request::Refer { name, .. }
            | Request::Delegate { name, .. } => *name,
        }
    }

    pub fn ack(&self) -> PieToken {
        match self {
            Request::Register { ack, .. }
            | Request::Refer { ack, .. }
            | Request::Delegate { ack, .. } => *ack,
        }
    }

    /// 编码为线上消息（**不写通道字段**：`[9..17]` 留给 [`Duet::encode`]）。
    pub fn encode(&self) -> [u8; LEN] {
        let (op, arg) = match self {
            Request::Register { session, .. } => (OP_REGISTER, session.get() as u64),
            Request::Refer { who, .. } => (OP_REFER, who.get() as u64),
            Request::Delegate { who, .. } => (OP_DELEGATE, who.get() as u64),
        };
        let mut msg = [0u8; LEN];
        msg[0] = op;
        msg[1..9].copy_from_slice(&arg.to_le_bytes());
        msg[17..LEN].copy_from_slice(self.name().bytes());
        msg
    }

    /// 解码：**长度、动词、名字**三者都要过（非法名不可表达——名字的构造义务在
    /// `Name::from_bytes` 上，这里只是把它接上）。
    pub fn decode(msg: &[u8]) -> Option<Request> {
        if msg.len() < LEN {
            return None;
        }
        let bytes: [u8; NAME_LEN] = msg[17..LEN].try_into().ok()?;
        let name = Name::from_bytes(bytes).ok()?;
        let arg = u64::from_le_bytes(msg[1..9].try_into().ok()?);
        let ack =
            PieToken::new(u64::from_le_bytes(msg[ACK_AT..ACK_AT + 8].try_into().ok()?) as usize);
        match msg[0] {
            OP_REGISTER => Some(Request::Register {
                name,
                session: PieToken::new(arg as usize),
                ack,
            }),
            OP_REFER => Some(Request::Refer {
                name,
                who: TaskId::new(arg as usize),
                ack,
            }),
            OP_DELEGATE => Some(Request::Delegate {
                name,
                who: TaskId::new(arg as usize),
                ack,
            }),
            _ => None,
        }
    }
}

/// 判据的回答：**成功** 或 **四种拒绝之一**（一字节，见模块头）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ack {
    Ok,
    Refused(Refused),
}

/// 四种拒绝。名字是权威的轴，线号是设备的轴——四种都发生在**名字**这一侧：
///
/// - `Unknown`：表里没有这个名字（设备树里它不是本控制器的中断源）；
/// - `Unclaimed`：行在，但 root 还没把它交给任何人（属主为空）；
/// - `NotYours`：名字有主，且不是请求者（**权威判定**，与"有没有人占着"无关）；
/// - `Taken`：名字是你的，但已经有一个活实例占着（旧实例没收线）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refused {
    Unknown,
    Unclaimed,
    NotYours,
    Taken,
}

impl Ack {
    pub const fn byte(self) -> u8 {
        match self {
            Ack::Ok => 0,
            Ack::Refused(Refused::Unknown) => 1,
            Ack::Refused(Refused::Unclaimed) => 2,
            Ack::Refused(Refused::NotYours) => 3,
            Ack::Refused(Refused::Taken) => 4,
        }
    }

    /// 一字节 → 值。不认识的取值 → `None`（**不假装成功**：调用方按失败处理）。
    pub const fn from_byte(b: u8) -> Option<Ack> {
        match b {
            0 => Some(Ack::Ok),
            1 => Some(Ack::Refused(Refused::Unknown)),
            2 => Some(Ack::Refused(Refused::Unclaimed)),
            3 => Some(Ack::Refused(Refused::NotYours)),
            4 => Some(Ack::Refused(Refused::Taken)),
            _ => None,
        }
    }
}

/// 回执的字节数（一字节）。
pub const ACK_LEN: usize = 1;

/// 报文对：一条中断线请求、一字节判据回答。回信地址写在 `[9..17]`（`ACK_AT`）。
impl Duet for Request {
    type Req = Request;
    type Rep = Ack;
    type Wire = [u8; LEN];

    const REQ: usize = LEN;
    const REP: usize = ACK_LEN;

    fn wire() -> [u8; LEN] {
        [0u8; LEN]
    }

    fn encode(req: &Request, at: PieToken, out: &mut [u8]) {
        out[..LEN].copy_from_slice(&req.encode());
        out[ACK_AT..ACK_AT + 8].copy_from_slice(&(at.get() as u64).to_le_bytes());
    }

    fn decode(buf: &[u8]) -> EnvResult<Ack> {
        Ack::from_byte(*buf.first().ok_or_else(denied)?).ok_or_else(denied)
    }
}
