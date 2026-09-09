//! handshake — 启动期握手：父域**开报到孔**、子域**靠泊**、两条报文。
//!
//! 为什么需要它：子域自建的孔要交给父域，父域配给的门闩要交给子域，而**第一次
//! 跨域通信必然要求一方发现一枚预置句柄**——子域要 pull 父域发来的 `Pier`，就得
//! 先找到那条通道。本模块的答案：
//!
//! - 父域 `dock()` 开一条 mtu = [`MTU`] 的**报到孔**（所有子域共用一条），`Hatch`
//!   **之前**给每个子域一份 `R|W` 副本；
//! - 子域 `moor()` 靠 `vestor == sire() && owner == sire()` 认出它——**冒充父域
//!   不可表达**（`vestor` 只能是门闩的直接授出者，`owner` 只能是资源的开辟者）。
//!
//! 两条报文各 8 字节 LE，**方向即类型**（不需要判别字节）：
//!
//! ```text
//! Quay  子 → 父：我自建孔在**父侧**的句柄（父域据此回 `Pier`）
//! Pier  父 → 子：目录请求门闩在**子侧**的句柄（0 = 无）
//! ```
//!
//! # 为什么报到孔与配给孔必须分开
//!
//! Hole 是**单槽**信箱：若子域既在一条孔上 push `Quay`、又在同一条上 pull `Pier`，
//! 它可能把自己刚写的 `Quay` 读回来（父域还没被调度），于是父域永远等不到报到——
//! 实测就是这样死锁的。故：
//!
//! - 子域在**父域开的**报到孔上 push `Quay`（父域只 pull 它）；
//! - 子域在**自己开的**孔上 pull `Pier`（父域只 push 它）。
//!
//! 这也正是既有 IPC 的形态：请求孔与回信孔是两条不同的孔。
//!
//! 通道用完不回收：`memo` 保活到关机，启动通道没有后续语义。

use env::{EnvError, EnvResult, make_err};

use crate::env::mail::{self, HolePie};

/// 报到孔与配给报文的单消息字节数：都只是单个 u64。
pub const MTU: usize = 8;

/// 枚举自己权限表的上限（防越界扫描跑飞；表量级个位数）。
const MAX_PIES: usize = 64;

const E_DENIED: isize = -1;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_DENIED))
}

/// 子 → 父：报到。`hole` = 我自建孔**在父侧**的句柄（`Accord` 的返回值）。
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

    /// 子侧：在报到孔上报到（阻塞至槽空）。
    pub fn push(self, quay: &HolePie) -> EnvResult<()> {
        quay.push(&self.hole.to_le_bytes())
    }

    /// 父侧：在报到孔上收报到（阻塞至有消息）。
    pub fn pull(quay: &HolePie) -> EnvResult<Quay> {
        let mut buf = [0u8; MTU];
        if quay.pull(&mut buf)? != MTU {
            return Err(denied());
        }
        Ok(Quay {
            hole: u64::from_le_bytes(buf),
        })
    }
}

/// 父 → 子：配给。`token` = 目录请求门闩**在子侧**的句柄（0 = 无）。
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

    /// 父侧：往子域自建的孔里配给（阻塞至槽空）。
    pub fn push(self, hole: &HolePie) -> EnvResult<()> {
        hole.push(&self.token.to_le_bytes())
    }

    /// 子侧：从自建孔里收配给（阻塞至有消息）。
    pub fn pull(hole: &HolePie) -> EnvResult<Pier> {
        let mut buf = [0u8; MTU];
        if hole.pull(&mut buf)? != MTU {
            return Err(denied());
        }
        Ok(Pier {
            token: u64::from_le_bytes(buf),
        })
    }
}

/// 父侧：开报到孔（一次，所有子域共用一条）。
///
/// **时序义务**：每个子域 `Hatch` 之前都要先 `accord` 它一份 `R|W` 副本。
pub fn dock() -> EnvResult<HolePie> {
    HolePie::unseal(MTU)
}

/// 子侧：认父域开的报到孔——本任务表里 `vestor == sire()` **且** `owner == sire()`
/// 的那一枚。
///
/// 两个条件缺一不可：`vestor` 说「这枚门闩是父域授给我的」，`owner` 说「这扇门是
/// 父域自己开的」。父域**转授**给我的门闩（如目录请求门闩）`vestor` 也是父域，但
/// 它的 `owner` 是别人——只看 `vestor` 会认错。
///
/// 顶级域（`sire() == 0`）没有报到孔 → `Denied`。
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
        // `vestor` 说「父域授给我的」；`owner` 说「这扇门是父域自己开的」。
        // 父域**转授**来的门闩（目录请求门闩）vestor 也是父域，但 owner 是别人。
        if let Ok((_, owner)) = mail::owned(token)
            && owner.get() == sire
        {
            return Ok(HolePie::from_token(token));
        }
    }
    Err(denied())
}
