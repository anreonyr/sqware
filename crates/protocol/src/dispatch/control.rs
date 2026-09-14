//! control — 父域 ↔ 目录**控制孔**上的两条报文（`Refer` / `Referred`）。
//!
//! 这两条不住在请求孔上：它们走的是**子域自建、交给父域**的那条控制孔（父 → dir），
//! 以及**父域开辟、交给子域**的那条上行孔（dir → 父）。用途是"引荐"：
//!
//! ```text
//! Refer     父 → dir  dir 控制孔   请把目录请求门闩的 R|W 授给 who（只引荐）
//! Reserve   父 → dir  dir 控制孔   同上，并**先把 name 记给 who**（引荐 + 预约）
//! Referred  dir → 父  dir 上行孔   已授（对方侧句柄；0 = 失败）
//! ```
//!
//! `Refer` / `Reserve` 是**同一条报文类型的两种线形**（tag 区分，无哨兵值）：预约必须
//! 早于开门——否则客户端拿到门闩即可注册，而预约还没落表。控制线程因此按
//! 「先 `reserve`、再 `accord`」处理。
//!
//! 线格式与启动期握手同形：`[0] = tag`、`[1..9] = payload u64 LE`；`Reserve` 多带一个
//! 名字（`REFER`）。tag 号沿用握手那一族的 3/4/5——**两条孔各自一套 tag 空间**，
//! 同号只是记着它们同源。
//!
//! **身份不进报文**：控制孔只有 root 推、上行孔只有子域推，"谁能推谁就是谁"是结构性的。
//!
//! 为什么住在 `protocol` 而不是 `runtime::core::handshake`：它们是**目录协议的**语义
//! （谁有资格注册哪个名字），而 `runtime` 是机制层——`protocol → runtime` 是单向边，
//! 机制层里躺着某个协议的报文正是那条边被违反的样子。

use env::{EnvError, EnvResult, NAME_LEN, Name, PieToken, TaskId, make_err};

use runtime::env::mail::HolePie;

/// 一条报文的字节数：1 字节 tag + 一个 u64。
/// 短报文（tag + 一个句柄）的长度与容量：`Quay`/`Referred` 这一族**就是** 9 字节。
const LEN: usize = 9;

/// `Reserve` 那条线形的**容量**：tag + who + 名字。
///
/// 它是容量不是长度——名字多长报文多长。故控制孔的接收侧按它备缓冲，**长度由内核给**。
///
/// 与目录询问同宽（`wire::CAP`）：控制报文那一格 `who` 站着询问里 `entry` 的位子，
/// 名字站着询问里名字的位子——**同一块缓冲给两条线形用**，故取后者（更宽的那个）。
const REFER: usize = super::wire::CAP;

/// 控制线程一次等待的上界（毫秒）：醒来看一眼"root 是不是已经走了"。
const WAIT_MS: usize = 100;

const TAG_REFER: u8 = 3;
const TAG_REFERRED: u8 = 4;
const TAG_RESERVE: u8 = 5;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 发一条报文：`[tag][payload]`。`payload` 收任何句柄——线形是 8 字节，但**类型不化**。
fn send(hole: &HolePie, tag: u8, payload: impl Into<usize>) -> EnvResult<()> {
    let mut buf = [0u8; LEN];
    buf[0] = tag;
    buf[1..].copy_from_slice(&payload.into().to_le_bytes());
    hole.push(&buf)
}

/// 收一条报文：读满 `LEN` 并校验 tag，返 payload。tag 不符 / 短读 → `Denied`。
///
/// **空孔是等，不是错**：`peek` 在空孔上返 `Busy`，而这条孔的读者是控制线程的
/// **唯一**等待点——把 `Busy` 当死会让它在两条请求之间的空档里自退（实测踩过）。
/// 「长度先问、再取」为的是长度由报文给（变长之后不再是固定 41 字节）。
fn recv(hole: &HolePie, tag: u8) -> EnvResult<usize> {
    let mut buf = [0u8; LEN];
    let len = loop {
        match hole.pull_timeout(&mut buf, WAIT_MS) {
            Ok(n) => break n,
            Err(e) if e.source.is_busy() => continue,
            Err(e) => return Err(e),
        }
    };
    if len != LEN || buf[0] != tag {
        return Err(denied());
    }
    Ok(usize::from_le_bytes(
        buf[1..].try_into().unwrap_or([0u8; 8]),
    ))
}

fn recv_pie(hole: &HolePie, tag: u8) -> EnvResult<PieToken> {
    recv(hole, tag).map(PieToken::new)
}

/// 父 → dir：引入请求。请把目录请求门闩的 `R|W` 授给 `who`；`name` 非空时
/// **先**把该名字预约给 `who`（两种线形，tag 区分）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refer {
    who: TaskId,
    name: Option<Name>,
}

impl Refer {
    /// 只引荐。
    pub fn new(who: TaskId) -> Self {
        Self { who, name: None }
    }

    /// 引荐 + 预约：`name` 从此归 `who`（只有它能注册这个名字）。
    pub fn named(who: TaskId, name: Name) -> Self {
        Self {
            who,
            name: Some(name),
        }
    }

    pub fn who(&self) -> TaskId {
        self.who
    }

    pub fn name(&self) -> Option<Name> {
        self.name
    }

    /// 父侧：往 dir 的控制孔里发引入请求。
    pub fn push(self, control: &HolePie) -> EnvResult<()> {
        match self.name {
            None => send(control, TAG_REFER, self.who.get()),
            Some(name) => {
                // 名字**多长写多长**：尾部那些字节不上线（补零是满帧时代的产物）。
                let text = name.text();
                let n = LEN + text.len();
                let mut buf = [0u8; REFER];
                buf[0] = TAG_RESERVE;
                buf[1..LEN].copy_from_slice(&self.who.get().to_le_bytes());
                buf[LEN..n].copy_from_slice(text);
                control.push(&buf[..n])
            }
        }
    }

    /// dir 控制线程：从控制孔里收引入请求（按 tag 分派线形）。
    ///
    /// 两种线形（`LEN` 与 `REFER`）长短不同，而孔不再替谁记"这条孔上的消息多大"
    /// ⇒ **长度先问、再取**，且空孔是**等**不是错（这是控制线程唯一的等待点）。
    /// 这一读与 [`recv`] 同一份规矩，只是它的取法定在 `REFER` 这一族上。
    pub fn pull(control: &HolePie) -> EnvResult<Refer> {
        // 缓冲按**容量**备（`REFER`），长度由内核给：两种线形长短不同，而孔不再替谁
        // 记"这条孔上的消息多大"。**有界等**——控制线程的唯一等待点不能是"探一下"，
        // 探空即退会把它在两条请求之间的空档里弄死（实测）。
        let mut buf = [0u8; REFER];
        let len = loop {
            match control.pull_timeout(&mut buf, WAIT_MS) {
                Ok(n) => break n,
                Err(e) if e.source.is_busy() => continue,
                Err(e) => return Err(e),
            }
        };
        if len < LEN || len > REFER {
            return Err(denied());
        }
        let who = || {
            TaskId(usize::from_le_bytes(
                buf[1..LEN].try_into().unwrap_or([0u8; 8]),
            ))
        };
        match (buf[0], len) {
            (TAG_REFER, LEN) => Ok(Refer::new(who())),
            (TAG_RESERVE, n) => {
                let text = core::str::from_utf8(&buf[LEN..n]).map_err(|_| denied())?;
                let name = Name::new(text).map_err(|_| denied())?;
                Ok(Refer::named(who(), name))
            }
            _ => Err(denied()),
        }
    }
}

/// dir → 父：已授。`token` = 对方侧句柄（0 = 失败）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Referred {
    token: PieToken,
}

impl Referred {
    pub fn new(token: impl Into<usize>) -> Self {
        Self {
            token: PieToken::new(token.into()),
        }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }

    /// dir 控制线程：把结果推回父域（dir 上行孔）。
    pub fn push(self, up: &HolePie) -> EnvResult<()> {
        send(up, TAG_REFERRED, self.token)
    }

    /// 父侧：在 dir 上行孔上收结果。
    pub fn pull(up: &HolePie) -> EnvResult<Referred> {
        Ok(Referred {
            token: recv_pie(up, TAG_REFERRED)?,
        })
    }
}
