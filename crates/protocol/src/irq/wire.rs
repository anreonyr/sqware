//! irq·wire — 中断线协议的线格式（纯函数，零依赖）。
//!
//! # 三个动词，一条记录
//!
//! ```text
//! [0]       op     u8        1 Register / 2 Refer / 3 Delegate
//! [1..9)    地址槽  u64 LE    回信孔在**驱动侧**的 token（传输字段，见下）
//! [9..17)   arg    u64 LE    Register：会话门闩在**驱动侧**的 token
//!                            Refer / Delegate：把名字交给谁（who）
//! [17..]    name   ≤ 31 字节 设备名（boot 给的设备树节点 basename）——**名字多长帧多长**
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
//! （`Taken`）是旧实例没收线。回信通道**由调用方自带**（报文里那个地址槽），与目录协议、
//! 控制台协议、他杀协议同一条规矩：谁发起谁备回信通道。
//!
//! # 投递载荷：u16 线号
//!
//! 驱动投给客户端的是**线号**（[`LINE_LEN`] 字节，`docs/driver.md` §5.3："长度是参数不是
//! 约定"——实测 `ndev = 95`，一个字节够，仍给 u16）。客户端核**来源**（`from` 是内核在
//! `Push` 时盖的章）与长度；线号本身它不认识，也不该认识。

use env::wire::NAME_LEN;
use env::{EnvError, EnvResult, Name, PieToken, TaskId, make_err};
use runtime::core::port::{ADDRESS_LEN, Duet, put_address};

/// 本协议的负码：无权 / 协议错（与内核码同表）。
pub(crate) fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 服务的名字（目录里登记的那一个）。
pub const SERVICE: &str = "plic";

/// 名字最长能写多少字节（内容上界：定长字段里那一个字节留给终止 NUL）。
pub const TEXT: usize = NAME_LEN - 1;

/// 一块内存要多大装得下任何一条帧：`op + 地址槽 + arg + 名字(上界)`。
pub const CAP: usize = 1 + ADDRESS_LEN + 8 + TEXT;

/// 投递载荷（线号）的字节数。
pub const LINE_LEN: usize = 2;

/// 正文里 `arg` 的偏移。
const ARG_AT: usize = 1 + ADDRESS_LEN;
/// 正文里名字的偏移。
const NAME_AT: usize = ARG_AT + 8;

const OP_REGISTER: u8 = 1;
const OP_REFER: u8 = 2;
const OP_DELEGATE: u8 = 3;

/// 一条请求：**动词 + 一条定长记录**。
///
/// 线形是统一的（见模块头），但两侧都用枚举说话：驱动不可能把 `who` 当成会话门闩读
/// ——那是解码时就已经分好的事。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Query {
    /// 登记：我按名字拿到的这台设备，会话门闩在那儿。
    Register { name: Name, session: PieToken },
    /// 写属主：这个名字归 `who`（**只有 root 会发它**）。
    Refer { name: Name, who: TaskId },
    /// 委托写权：**这个名字的属主，从此也可以由 `who` 写**（同样只有 root 会发它）。
    ///
    /// 为什么需要它：root 的服务重启落在**它自己域里的监管线程**上（`docs/root.md` §5.3），
    /// 而线程不是域——驱动认的是"推者是不是我的 `sire`"，监管线程答不上这一条。放弃这条
    /// 判据不行（那等于谁都能改属主）；把判据放宽成"同一个域"也不行（驱动拿不到推者的域）。
    /// 于是让 **root 亲口把这份写权委托给它信任的那个线程**：判据从"谁的 id"变成
    /// "root 有没有为这个名字委托过你"——一条可审计、可收回（重启即失效）的授权。
    Delegate { name: Name, who: TaskId },
}

impl Query {
    /// 三个构造器都**不收回信地址**：它是**传输字段**（地址槽），由 [`Duet::encode`]
    /// 按本端回信孔填——客户端手里没有那枚 token，故请求值里没有这一格。
    pub const fn register(name: Name, session: PieToken) -> Query {
        Query::Register { name, session }
    }

    pub const fn refer(name: Name, who: TaskId) -> Query {
        Query::Refer { name, who }
    }

    pub const fn delegate(name: Name, who: TaskId) -> Query {
        Query::Delegate { name, who }
    }

    pub fn name(&self) -> Name {
        match self {
            Query::Register { name, .. }
            | Query::Refer { name, .. }
            | Query::Delegate { name, .. } => *name,
        }
    }

    /// 这一帧多少字节：头 17 + 名字那一段。
    pub fn len(&self) -> usize {
        NAME_AT + self.name().text().len()
    }

    /// 编码为线上消息（**不写地址槽**：留给 [`Duet::encode`]；尾部不补零）。
    pub fn encode(&self) -> ([u8; CAP], usize) {
        let (op, arg) = match self {
            Query::Register { session, .. } => (OP_REGISTER, session.get() as u64),
            Query::Refer { who, .. } => (OP_REFER, who.get() as u64),
            Query::Delegate { who, .. } => (OP_DELEGATE, who.get() as u64),
        };
        let mut msg = [0u8; CAP];
        msg[0] = op;
        msg[ARG_AT..ARG_AT + 8].copy_from_slice(&arg.to_le_bytes());
        let name = self.name();
        let text = name.text();
        let n = self.len();
        msg[NAME_AT..n].copy_from_slice(text);
        (msg, n)
    }

    /// 解码：**长度、动词、名字**三者都要过（非法名不可表达——名字的构造义务在
    /// `Name::from_slice` 上，这里只是把它接上）。
    pub fn decode(msg: &[u8]) -> Option<Query> {
        let name = Name::from_slice(msg.get(NAME_AT..)?).ok()?;
        let arg = u64::from_le_bytes(msg.get(ARG_AT..ARG_AT + 8)?.try_into().ok()?);
        match *msg.first()? {
            OP_REGISTER => Some(Query::Register {
                name,
                session: PieToken::new(arg as usize),
            }),
            OP_REFER => Some(Query::Refer {
                name,
                who: TaskId::new(arg as usize),
            }),
            OP_DELEGATE => Some(Query::Delegate {
                name,
                who: TaskId::new(arg as usize),
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

/// 报文对：一条中断线询问、一字节判据应答。地址槽在 `[1..9)`，**每条询问都带**
/// （本协议每请求一趟：`irq::client` 的 `Line::ask` 每次开一枚新回信孔）。
impl Duet for Query {
    type Req = Query;
    type Rep = Ack;
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

    fn decode(buf: &[u8]) -> EnvResult<Ack> {
        Ack::from_byte(*buf.first().ok_or_else(denied)?).ok_or_else(denied)
    }
}
