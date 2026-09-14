//! handshake — 启动期握手：父域**开上行孔**、子域**靠泊**、两条报文。
//!
//! "引荐"那一对报文（`Refer` / `Referred`）住 `protocol::dispatch::control`：它们是
//! 目录协议的语义，不是机制。
//!
//! 目标（B 版）：**父域 root 手里零服务孔**。子域自建**控制孔**并把父侧句柄交给
//! root；客户端要用的目录门闩由 **dir 亲授**（root 只转达「授给谁」）——客户端拿到
//! 的副本 `vestor == dir`，来源可自证。
//!
//! 线格式：`[0] = tag`、`[1..9] = payload u64 LE`——**定长 9 字节**，两条报文同一形状。
//!
//! ```text
//! Quay      子 → 父   子域上行孔      我控制孔在父侧的句柄
//! Pier      父 → 子   子域下行孔      目录门闩在本侧的句柄（0 = 无）
//! ```
//!
//! **每条孔单一发送者**：上行孔只有子域推、下行孔只有父域推、dir 控制孔只有 root 推。
//! 「谁能推谁就是谁」是结构性的，故报文里不带任何身份。
//!
//! 为什么上行孔与下行孔必须分开：Hole 是单槽信箱，同一条孔上既 push 又 pull 会把
//! 自己刚写的消息读回来（T2 实测死锁）。这也正是既有 IPC 的形态：请求孔与回信孔
//! 是两条不同的孔。
//!
//! 通道用完不回收：门闩由各任务的权限表保活到关机，启动通道没有后续语义。

use env::{EnvError, EnvResult, PieToken, TaskId, make_err};

use crate::core::port::{Access, Policy, ship};
use crate::env::mail::{self, HolePie};

/// 报文长度：1 字节 tag + 一个 u64（本模块自用：两条报文的线形都定长）。
///
/// **它就是长度**：本模块的报文没有变长那一段，故这里一个数既是容量也是被推的字节数
/// （与各协议不同——那边 `CAP` 只是容量上界，长度由成帧的人报）。
const LEN: usize = 9;

const TAG_QUAY: u8 = 1;
const TAG_PIER: u8 = 2;

/// 枚举自己权限表的上限（防越界扫描跑飞；表量级个位数）。
const MAX_PIES: usize = 64;

const E_DENIED: isize = -1;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_DENIED))
}

/// 发一条报文：`[tag][payload]`。
///
/// `payload` 收任何句柄：线形是 8 字节，但**类型不化**——调用方继续拿 `PieToken`。
fn send(hole: &HolePie, tag: u8, payload: impl Into<usize>) -> EnvResult<()> {
    let mut buf = [0u8; LEN];
    buf[0] = tag;
    buf[1..].copy_from_slice(&payload.into().to_le_bytes());
    hole.push(&buf)
}

/// 收一条报文：读满 `LEN` 并校验 tag，返 payload。tag 不符 / 短读 → `Denied`。
fn recv_pie(hole: &HolePie, tag: u8) -> EnvResult<PieToken> {
    recv(hole, tag).map(PieToken::new)
}

fn recv(hole: &HolePie, tag: u8) -> EnvResult<usize> {
    let mut buf = [0u8; LEN];
    // 长度由内核给（谁推的谁知道多长），本模块只认"[tag][u64]"这一种形状。
    if hole.pull(&mut buf)? != LEN || buf[0] != tag {
        return Err(denied());
    }
    Ok(usize::from_le_bytes(
        buf[1..].try_into().unwrap_or([0u8; 8]),
    ))
}

/// 子 → 父：报到。`hole` = 我自建控制孔**在父侧**的句柄（`Accord` 的返回值）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Quay {
    hole: PieToken,
}

impl Quay {
    pub fn new(hole: impl Into<usize>) -> Self {
        Self {
            hole: PieToken::new(hole.into()),
        }
    }

    pub fn hole(&self) -> PieToken {
        self.hole
    }

    /// 子侧：在子域上行孔上报到。
    pub fn push(self, up: &HolePie) -> EnvResult<()> {
        send(up, TAG_QUAY, self.hole)
    }

    /// 父侧：在子域上行孔上收报到。
    pub fn pull(up: &HolePie) -> EnvResult<Quay> {
        Ok(Quay {
            hole: recv_pie(up, TAG_QUAY)?,
        })
    }
}

/// 父 → 子：配给。`token` = 目录门闩**在子侧**的句柄（0 = 无）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pier {
    token: PieToken,
}

impl Pier {
    pub fn new(token: impl Into<usize>) -> Self {
        Self {
            token: PieToken::new(token.into()),
        }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }

    /// 父侧：往子域下行孔里配给。
    pub fn push(self, down: &HolePie) -> EnvResult<()> {
        send(down, TAG_PIER, self.token)
    }

    /// 子侧：从下行孔里收配给。
    pub fn pull(down: &HolePie) -> EnvResult<Pier> {
        Ok(Pier {
            token: recv_pie(down, TAG_PIER)?,
        })
    }
}

/// 父侧：为子域开上行孔并把 `R|W|VEST` 副本授给它。
///
/// **带 `VEST`**：子域要把这条孔再授给自己的**控制线程**（同域跨 task 门闩不共享），
/// 而 `Accord` 的门槛是"源门闩持 `VEST`"。
///
/// **时序义务**：必须早于 `Hatch(child)`——否则子域起跑时 `moor()` 找不到它。
pub fn dock(child: TaskId) -> EnvResult<HolePie> {
    let up = HolePie::unseal()?;
    ship(&up, child, Access::READ | Access::WRITE, Policy::VEST)?;
    Ok(up)
}

/// 子侧：认父域给的上行孔——本任务表里 `vestor == sire()` **且** `owner == sire()`
/// 的那一枚。
///
/// 两个条件缺一不可：`vestor` 说「这枚门闩是父域授给我的」，`owner` 说「这扇门是
/// 父域自己开的」。父域**转授**来的门闩（如目录请求门闩）`vestor` 也是父域，但
/// 它的 `owner` 是别人——只看 `vestor` 会认错。
///
/// 顶级域（`sire() == 0`）没有上行孔 → `Denied`。
pub fn moor() -> EnvResult<HolePie> {
    let sire = crate::env::task::sire()?.get();
    if sire == 0 {
        return Err(denied());
    }
    for index in 0..MAX_PIES {
        let (token, _, vestor) = mail::collect(index)?;
        if token.get() == 0 {
            break;
        }
        if vestor.get() != sire {
            continue;
        }
        if let Ok((_, owner)) = mail::reserve(token)
            && owner.get() == sire
        {
            return Ok(HolePie::from_token(token.get()));
        }
    }
    Err(denied())
}
