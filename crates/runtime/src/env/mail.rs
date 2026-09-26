//! Mail 域 —— **通信面**：门闩（pie）那一族"怎么用"的那一半。
//!
//! # 两条轴，两个文件
//!
//! `env::fid` 文件头把 `PieCall`（class 7）与 `MailCall`（class 5）立成两条正交的轴
//! （权柄 / 数据）。本层按轴分文件：
//!
//!   - **通信面（本文件）**：class 5 的 `Push` / `Pull` / `Wait` / `Hush` / `Ring`，加
//!     class 9（`ToleCall`：一枚"组"的造 / 挂 / 摘 / 等）。组是**多路等待**——成员是孔的
//!     一个方向或一枚铃，故它接着本文件那一族的等待语义（分界见 `env::fid`：
//!     "本类不搬载荷"）；
//!   - **权柄面**（[`pie`](super::pie)）：class 7 的转发 + [`AnyPie`] 与它的四份实现。
//!
//! # 四种资源的用户态句柄都住本文件
//!
//! Hole（数据面）、Nole（门铃）、Pole（页视图）、Tole（组）——它们的**权柄面**
//! （`Seal` / `Narrow` / `Accord` / `Revoke` / `Release`）由 [`AnyPie`] 提供，四份实现
//! 集中在 `pie.rs`。口径一句话：**句柄一处（本文件），权柄动词一处（`pie.rs`）**。
//!
//! Pole 不属通信面，它住这里是因为"句柄一处"这一条：它的数据面是页视图
//! （`open` / `shut`），而页视图与权柄一样，与"往哪推、从哪收"无关。
//!
//! # 名字照旧（本文件为什么有一大段 `pub use`）
//!
//! 权柄轴拆去 `pie.rs` 之后，`runtime::env::mail::X` 这一形（`protocol` / `programs` /
//! `harness` 里十几处调用点）**一行没改**：下面把 `pie.rs` 的每一项按名字转出去。
//! 这是本仓搬家的既有先例（`Access`/`Policy`、`Announce`/`Grant`/`Died` 都是这么转的）。
//!
//! push/pull 的阻塞：内核 Push/Pull 槽满/槽空返 `-3 Busy`；本层转 `Wait` 原语
//! 挂起（让出 CPU），被对侧唤醒后重试——真阻塞，不占核。

use env::Wait;
use env::{
    EnvResult, HoleDir, MailCall, MailCallRet, Mark, PieResult, PieToken, TaskId, ToleResult,
    VirtAddr,
};

/// 单调时钟读数（纳秒）——`pull_timeout` 的 deadline 用（机器无关，不依赖
/// timebase 频率）。内核那一格没有失败支，故跟着 [`clock`](crate::env::chrono::clock)
/// 一起不返 `EnvResult`。
fn now_ns() -> u64 {
    crate::env::chrono::clock()
}

// ── 权柄轴（class 7）搬去 `pie.rs` 之后的名字照旧 ──────────────────────────
//
// 整面转出（**不挑**）：转发是"路径不变"的保证，一旦按"今天谁在用"挑，下一个调用点就得
// 先认出这层壳才知道自己该写 `pie::`——那正是这一层想免掉的认知成本。
pub use super::pie::{
    AnyPie, Pie, Pies, accord, collect, narrow, open, pies, release, reserve, revoke, seal, shut,
    table_size, unseal_hole, unseal_nole, unseal_pole,
};

// ── 裸函数层（envcall 转发，零业务逻辑）：class 5（数据轴）──

/// push 一条消息（`msg[..len]` 进 hole 槽）。`len ∈ 1..=一页`（破了界答 `Denied`）。
pub fn push(token: PieToken, msg: *const u8, len: usize) -> EnvResult<()> {
    let r = MailCall::Push {
        token,
        msg: VirtAddr::new(msg as usize),
        len,
    }
    .call()?;
    match r {
        MailCallRet::Push(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// pull 一条消息（最多装 `buf[..max]`）。返实际长度（≤ max）；发送者丢弃。
/// 装不下返 `Denied` 且槽原样；**给一页就装得下任何一条消息**（载体封顶一页）。
/// 要问长度用 [`pull_len`]。
pub fn pull(token: PieToken, buf: *mut u8, max: usize) -> EnvResult<usize> {
    pull_from(token, buf, max).map(|(n, _)| n)
}

/// 只问长度（**不动槽**）：返槽里那条消息的长度与发送者，一个字节都不取。
///
/// 走 `Pull { max: 0 }`——与 `Wait::POLL`「只探测不挂起」同一形状的"只问"。
/// **不是取消息的前一步**（那一步由载体的界接手：一页缓冲一趟取走）；它的读者是
/// "等之前先看一眼"那一格（`harness` 的 waiter）。
pub fn pull_len(token: PieToken) -> EnvResult<(usize, TaskId)> {
    pull_from(token, core::ptr::null_mut(), 0)
}

/// pull 一条消息并取回**发送者**（`(长度, 发送者 TaskId)`）。
///
/// 发送者由内核在 `Push` 时盖章——身份不可伪造，不必再从报文里猜。
/// `max == 0` ⇒ 只报长度、不动槽（收方缓冲不参与）。
pub fn pull_from(token: PieToken, buf: *mut u8, max: usize) -> EnvResult<(usize, TaskId)> {
    let r = MailCall::Pull {
        token,
        buf: VirtAddr::new(buf as usize),
        max,
    }
    .call()?;
    match r {
        MailCallRet::Pull((n, from)) => Ok((n, from)),
        _ => unreachable!(),
    }
}

/// 等 hole 某方向就绪：`millis`（上限族，`Wait`）。
/// 返回 `true` = 本次调用当场就绪；`false` = 未就绪（探测失败，或挂起过）。
///
/// 门铃（Nole）也走这一个：`dir` 必须给 [`HoleDir::Pull`]——铃只有"响了"一条方向，
/// 别的值内核答 `Denied`。裸函数层不为它另开一个名字：`Bell::wait` 就是这一句。
pub fn wait(token: PieToken, dir: HoleDir, millis: Wait) -> EnvResult<bool> {
    let r = MailCall::Wait { token, dir, millis }.call()?;
    match r {
        MailCallRet::Wait(ready) => Ok(ready),
        _ => unreachable!(),
    }
}

/// 响铃（门铃专用）：置"有待取之事"并唤醒听者。已响 → `Busy`。
pub fn ring(token: PieToken) -> EnvResult<()> {
    let r = MailCall::Ring { token }.call()?;
    match r {
        MailCallRet::Ring(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 应铃（门铃专用）：清掉"有待取之事"，内核随即重开本 hart 的中断闸门。
pub fn hush(token: PieToken) -> EnvResult<()> {
    let r = MailCall::Hush { token }.call()?;
    match r {
        MailCallRet::Hush(()) => Ok(()),
        _ => unreachable!(),
    }
}

// ── 裸函数层（envcall 转发，零业务逻辑）：class 9（多路等待）──

/// 造一个空组 → 组的句柄。
///
/// `shared` = 这枚组允不许多个使用者（**造的时候定、之后不可变**，见 `env::fid` 的
/// `ToleCall::Unseal`）：`false` = 独占组（授出即移交、复制不出来），`true` = 共享组
/// （可 `accord` 复制给多个任务；组键的唤醒是提示型——放行全链）。
pub fn unseal(shared: bool) -> ToleResult<PieToken> {
    env::tole::unseal(shared)
}

/// 把 `pie` 的一个方向挂进 `tole`（同成员幂等）。
pub fn attach(tole: PieToken, pie: PieToken, dir: HoleDir) -> ToleResult<()> {
    env::tole::attach(tole, pie, dir)
}

/// 从 `tole` 摘掉一格；没挂过即无事。
pub fn detach(tole: PieToken, pie: PieToken, dir: HoleDir) -> ToleResult<()> {
    env::tole::detach(tole, pie, dir)
}

/// 等到组里任意一格有事：`(哪一枚, 哪个方向)`；`millis` 上限族，同全树。
///
/// `PieToken::NONE` = 没等到（或挂起过——见 `env::fid` 的 `ToleCall::Await`）。
/// 这一格是**裸函数层**：把"没等到"翻成 `Option` 的是 `crate::core::pile::Pile`。
pub fn await_(tole: PieToken, millis: Wait) -> ToleResult<(PieToken, HoleDir)> {
    env::tole::await_(tole, millis)
}

// ── 四种资源的用户态句柄 ──────────────────────────────────────────────────

/// Hole 门闩用户态句柄——**数据面那一枚**。
///
/// 它的权柄面不在这里：`Seal` / `Narrow` / `Accord` / `Revoke` / `Release` 由 `pie.rs`
/// 的 [`AnyPie`] impl 提供（见本文件头注）。这里只有构造与数据面。
pub struct HolePie {
    token: PieToken,
}

impl HolePie {
    /// 解封 Hole：**记号必填**（`mark` = 这枚孔干什么用的，见 [`unseal_hole`]）。
    pub fn unseal(mark: Mark) -> PieResult<Self> {
        Ok(Self {
            token: super::pie::unseal_hole(mark)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 等某方向就绪：`millis`（上限族，`Wait`）。
    /// 返回 `true` = 调用当场就绪；`false` = 未就绪（探测失败，或挂起过）。
    pub fn wait(&self, dir: HoleDir, millis: Wait) -> EnvResult<bool> {
        wait(self.token, dir, millis)
    }

    /// 写消息（**`1..=一页`**）：槽满则睡到有空间（让出 CPU）。
    pub fn push(&self, msg: &[u8]) -> EnvResult<()> {
        loop {
            match push(self.token, msg.as_ptr(), msg.len()) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Push, Wait::Forever)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 取消息：槽空则睡到有信（让出 CPU）。返实际收到字节数（≤ `buf.len()`）。
    ///
    /// **装不下（消息比 `buf` 长）返 `Denied`，且槽原样**——给一页就装得下任何一条消息
    /// （载体封顶一页），故这一支只会发生在**你自己给得更小**的时候；真给了小缓冲又不想丢，
    /// 先问 [`HolePie::peek`] 再备够。这里不替调用方把槽丢掉：丢一条消息是不可逆的。
    pub fn pull(&self, buf: &mut [u8]) -> EnvResult<usize> {
        loop {
            match pull(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(n) => return Ok(n),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Pull, Wait::Forever)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 同 [`HolePie::pull`]，但一并取回**发送者**（内核盖章的 task id）。
    pub fn pull_from(&self, buf: &mut [u8]) -> EnvResult<(usize, TaskId)> {
        loop {
            match pull_from(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(v) => return Ok(v),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Pull, Wait::Forever)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 只看一眼：槽里那条消息的**长度与发送者**，**一个字节都不取**（槽留原样）。
    ///
    /// **不是取消息的前一步**：载体封顶一页 ⇒ 一座一页缓冲一趟取走任何一条消息。
    /// 它的读者是"等之前先看一眼"那一格（`harness` 的 waiter）。
    /// 槽空 → `Err(Busy)`（没有可取之事，与 `pull` 同一个码）。
    pub fn peek(&self) -> EnvResult<(usize, TaskId)> {
        pull_len(self.token)
    }

    /// 有界 pull：槽空则最多等 `millis`；仍无消息 → `Err(Busy)`（码 -3）。
    /// 用于「等对端回复」这类必须有上界的往返：无限等会把协议错误（回复被丢弃、
    /// 对端漏回）变成不可诊断的挂起。**超时后该 hole 不再"干净"**——迟到的回复
    /// 仍可能落进槽里，使下一次 pull 取到上一条；调用方应弃用该会话。
    ///
    /// 实现要点：`wait` 返 false **不等于**超时——它可能是「唤醒闩（pend）被消费」
    /// 或一次无关唤醒（见 `messenger::wake`：无等待者时置 pend，而成功裸 pull 不会
    /// 消费它，故 pend 可能是陈旧的）。所以这里按 **deadline 循环**：只有 `clock()`
    /// 真的走完 `millis` 才报 Busy，否则带着剩余时间重试。
    pub fn pull_timeout(&self, buf: &mut [u8], millis: Wait) -> EnvResult<usize> {
        self.pull_timeout_from(buf, millis).map(|(n, _)| n)
    }

    /// 同 [`HolePie::pull_timeout`]，但一并取回**发送者**——「有界等」与「认来源」
    /// 是同一次收的两个事实，分成两趟取会把竞态留在中间。
    pub fn pull_timeout_from(&self, buf: &mut [u8], millis: Wait) -> EnvResult<(usize, TaskId)> {
        // **永久那一格在这里落成一个"到不了的点"，不落成 `Wait::Forever`**——照实记：内核的
        // 武装点被 `min(最近活到点, chrono::timer::BLIND_MS)` 收着（`chrono/timer.rs`），故
        // "一个到不了的点" = **每 ~100 ms 被叫醒一次、自己复探**；那一层复探是这条等待今天的
        // 护栏（`kernel/src/work/room/messenger/wait/mod.rs` 记着"`await_(…::MAX)` 的板线程
        // 永远不醒"那条实测）。落成真永久 = 不武装定时器，要先动内核那一格——**不在这一刀里**。
        let deadline = match millis {
            Wait::Forever => u64::MAX,
            Wait::AtMost(ms) => now_ns().saturating_add((ms as u64).saturating_mul(1_000_000)),
        };
        loop {
            match pull_from(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(v) => return Ok(v),
                Err(e) if e.source.is_busy() => {
                    let now = now_ns();
                    if now >= deadline {
                        return pull_from(self.token, buf.as_mut_ptr(), buf.len());
                    }
                    let remain_ms = ((deadline - now) / 1_000_000).max(1) as usize;
                    let _ = self.wait(HoleDir::Pull, Wait::AtMost(remain_ms))?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// Nole 门闩用户态句柄——**门铃**：四枚句柄里唯一没有自己那一套动词的那种（它只有构造与
/// [`AnyPie`] 那一套；`HolePie` 多数据面、`PolePie` 多页视图、`TolePie` 多挂摘等）。
///
/// 它没有 `push`/`pull`（那是 Hole 的数据面）、没有 `open`/`shut`（那是 Pole 的
/// 页视图）。它能做的只有 [`AnyPie`] 那一套（`accord`/`narrow`/`revoke`/`release`
/// /`seal`）——因为它的全部内容就是"我持有这一枚"。
///
/// **听与应不在这枚类型上**：`Bell::wait` / `Bell::hush` 就是裸函数 [`wait`] / [`hush`]
/// 那两句（见 `crate::core::bell`）——铃只有一条方向，故不另开名字。
pub struct NolePie {
    token: PieToken,
}

impl NolePie {
    /// 解封一枚 Nole（无参数：没有大小、没有对齐）。
    pub fn unseal() -> PieResult<Self> {
        Ok(Self {
            token: unseal_nole()?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// Pole 门闩用户态句柄——**页视图那一枚**。
pub struct PolePie {
    token: PieToken,
}

impl PolePie {
    /// 解封 Pole：一段页级安全内存（**大小页对齐**，清零）。
    ///
    /// 创建者的视图由内核顺手落好（`unseal` 内部 `auto-map`），但**那个 VA 不在这里回**
    /// ——要地址就再 `open` 一次（幂等，返同一个 VA）。
    pub fn unseal(size: usize) -> PieResult<Self> {
        Ok(Self {
            token: unseal_pole(size)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 开闩：借映进本任务空间 → `(视图起点, 这一段多大)`（同 token 幂等复用）。
    ///
    /// 薄层只封这一次 envcall；"视图"这个用法在 [`crate::core::dock`]。
    pub fn open(&self) -> PieResult<(usize, usize)> {
        open(self.token)
    }

    pub fn shut(&self) -> PieResult<()> {
        shut(self.token)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// 组的用户态句柄（与 [`HolePie`] 同款：只持一枚号，资源实体在内核）。
///
/// 方法集 = 这一个对象上能做的三件事（`unseal` 是**构造**，做不成 `&self` 方法）。
pub struct TolePie {
    token: PieToken,
}

impl TolePie {
    /// 造一个空组（种类见 [`unseal`]）。
    pub fn unseal(shared: bool) -> ToleResult<Self> {
        Ok(Self {
            token: unseal(shared)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的组）。
    pub fn from_token(token: PieToken) -> Self {
        Self { token }
    }

    /// 把一枚成员的一个方向挂进来。
    pub fn attach<M: Mate>(&self, mate: &M, dir: HoleDir) -> ToleResult<()> {
        attach(self.token, mate.token(), dir)
    }

    /// 摘掉一格。
    pub fn detach<M: Mate>(&self, mate: &M, dir: HoleDir) -> ToleResult<()> {
        detach(self.token, mate.token(), dir)
    }

    /// 等到任意一格有事。
    pub fn await_(&self, millis: Wait) -> ToleResult<(PieToken, HoleDir)> {
        await_(self.token, millis)
    }

    pub fn token(&self) -> PieToken {
        self.token
    }
}

/// 能当**一格成员**的东西：孔与铃。
///
/// 与内核侧 `mail::tole::Mate` 是同一条边界：页不进组（没有"有事"这回事），组也不
/// 进组（没有位，判据会变成沿图的递归）。用一个 trait 而不是收 `PieToken`，是为了
/// 让"能挂什么"在编译期就说得清。
///
/// `token` 不在 [`AnyPie`] 里（那一位是"表示层转换，与 ABI 无关"）；
/// 本 trait 的存在理由正是要那个号，故它自带一支。
pub trait Mate {
    /// 本成员在**我这张表**里的号。
    fn token(&self) -> PieToken;
}

impl Mate for HolePie {
    fn token(&self) -> PieToken {
        HolePie::token(self)
    }
}

impl Mate for NolePie {
    fn token(&self) -> PieToken {
        NolePie::token(self)
    }
}
