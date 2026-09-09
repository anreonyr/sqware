//! handshake — 启动期握手：父域**开上行孔**、子域**靠泊**、四条报文。
//!
//! 目标（B 版）：**父域 root 手里零服务孔**。子域自建**控制孔**并把父侧句柄交给
//! root；客户端要用的目录门闩由 **dir 亲授**（root 只转达「授给谁」）——客户端拿到
//! 的副本 `vestor == dir`，来源可自证。
//!
//! 线格式：`[0] = tag`、`[1..9] = payload u64 LE`，孔 mtu = [`MTU`]。
//!
//! ```text
//! Quay      子 → 父   子域上行孔      我控制孔在父侧的句柄
//! Pier      父 → 子   子域下行孔      目录门闩在本侧的句柄（0 = 无）
//! Refer     父 → dir  dir 控制孔      请把目录门闩的 R|W 授给 who
//! Referred  dir → 父  dir 上行孔      已授（对方侧句柄；0 = 失败）
//! ```
//!
//! **每条孔单一发送者**：上行孔只有子域推、下行孔只有父域推、dir 控制孔只有 root 推。
//! 「谁能推谁就是谁」是结构性的，故报文里不带任何身份。
//!
//! 为什么上行孔与下行孔必须分开：Hole 是单槽信箱，同一条孔上既 push 又 pull 会把
//! 自己刚写的消息读回来（T2 实测死锁）。这也正是既有 IPC 的形态：请求孔与回信孔
//! 是两条不同的孔。
//!
//! 通道用完不回收：`memo` 保活到关机，启动通道没有后续语义。

use env::{EnvError, EnvResult, Permission, TaskId, make_err};

use crate::env::mail::{self, HolePie};

/// 报文长度：1 字节 tag + 一个 u64。
pub const MTU: usize = 9;

const TAG_QUAY: u8 = 1;
const TAG_PIER: u8 = 2;
const TAG_REFER: u8 = 3;
const TAG_REFERRED: u8 = 4;

/// 枚举自己权限表的上限（防越界扫描跑飞；表量级个位数）。
const MAX_PIES: usize = 64;

const E_DENIED: isize = -1;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_DENIED))
}

/// 发一条报文：`[tag][payload]`。
fn send(hole: &HolePie, tag: u8, payload: u64) -> EnvResult<()> {
    let mut buf = [0u8; MTU];
    buf[0] = tag;
    buf[1..].copy_from_slice(&payload.to_le_bytes());
    hole.push(&buf)
}

/// 收一条报文：读满 `MTU` 并校验 tag，返 payload。tag 不符 / 短读 → `Denied`。
fn recv(hole: &HolePie, tag: u8) -> EnvResult<u64> {
    let mut buf = [0u8; MTU];
    if hole.pull(&mut buf)? != MTU || buf[0] != tag {
        return Err(denied());
    }
    Ok(u64::from_le_bytes(buf[1..].try_into().unwrap_or([0u8; 8])))
}

/// 子 → 父：报到。`hole` = 我自建控制孔**在父侧**的句柄（`Accord` 的返回值）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Quay {
    hole: u64,
}

impl Quay {
    pub fn new(hole: u64) -> Self {
        Self { hole }
    }

    pub fn hole(&self) -> u64 {
        self.hole
    }

    /// 子侧：在子域上行孔上报到。
    pub fn push(self, up: &HolePie) -> EnvResult<()> {
        send(up, TAG_QUAY, self.hole)
    }

    /// 父侧：在子域上行孔上收报到。
    pub fn pull(up: &HolePie) -> EnvResult<Quay> {
        Ok(Quay {
            hole: recv(up, TAG_QUAY)?,
        })
    }
}

/// 父 → 子：配给。`token` = 目录门闩**在子侧**的句柄（0 = 无）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pier {
    token: u64,
}

impl Pier {
    pub fn new(token: u64) -> Self {
        Self { token }
    }

    pub fn token(&self) -> u64 {
        self.token
    }

    /// 父侧：往子域下行孔里配给。
    pub fn push(self, down: &HolePie) -> EnvResult<()> {
        send(down, TAG_PIER, self.token)
    }

    /// 子侧：从下行孔里收配给。
    pub fn pull(down: &HolePie) -> EnvResult<Pier> {
        Ok(Pier {
            token: recv(down, TAG_PIER)?,
        })
    }
}

/// 父 → dir：引入请求。请把目录请求门闩的 `R|W` 授给 `who`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refer {
    who: TaskId,
}

impl Refer {
    pub fn new(who: TaskId) -> Self {
        Self { who }
    }

    pub fn who(&self) -> TaskId {
        self.who
    }

    /// 父侧：往 dir 的控制孔里发引入请求。
    pub fn push(self, control: &HolePie) -> EnvResult<()> {
        send(control, TAG_REFER, self.who.get() as u64)
    }

    /// dir 控制线程：从控制孔里收引入请求。
    pub fn pull(control: &HolePie) -> EnvResult<Refer> {
        Ok(Refer {
            who: TaskId(recv(control, TAG_REFER)? as usize),
        })
    }
}

/// dir → 父：已授。`token` = 对方侧句柄（0 = 失败）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Referred {
    token: u64,
}

impl Referred {
    pub fn new(token: u64) -> Self {
        Self { token }
    }

    pub fn token(&self) -> u64 {
        self.token
    }

    /// dir 控制线程：把结果推回父域（dir 上行孔）。
    pub fn push(self, up: &HolePie) -> EnvResult<()> {
        send(up, TAG_REFERRED, self.token)
    }

    /// 父侧：在 dir 上行孔上收结果。
    pub fn pull(up: &HolePie) -> EnvResult<Referred> {
        Ok(Referred {
            token: recv(up, TAG_REFERRED)?,
        })
    }
}

/// 父侧：为子域开上行孔并把 `R|W|VEST` 副本授给它。
///
/// **带 `VEST`**：子域要把这条孔再授给自己的**控制线程**（同域跨 task 门闩不共享），
/// 而 `Accord` 要求源门闩有 `VEST|BACK`。
///
/// **时序义务**：必须早于 `Hatch(child)`——否则子域起跑时 `moor()` 找不到它。
pub fn dock(child: TaskId) -> EnvResult<HolePie> {
    let up = HolePie::unseal(MTU)?;
    up.accord(
        child.get(),
        Permission::READ | Permission::WRITE | Permission::VEST,
    )?;
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
        if token == 0 {
            break;
        }
        if vestor.get() != sire {
            continue;
        }
        if let Ok((_, owner)) = mail::owned(token)
            && owner.get() == sire
        {
            return Ok(HolePie::from_token(token));
        }
    }
    Err(denied())
}
