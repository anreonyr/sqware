//! session — 一条会话的**公共骨架**：谁、往哪回、凭什么说它还在。
//!
//! 它不是哪一条协议的私事：console 有会话表，dispatch/doom/irq/uart 是一问一答，但
//! **判活**与**收场**两件事五家都要。故它住 `protocol` 顶层——与 `startup` 同一条理由：
//! `lib.rs` 头注那张"四个协议"表说的是"某一条协议的私事"，本件不是。
//!
//! # 三格，各指回一个操作
//!
//! ```text
//! peer   谁（内核在 Push 时盖章，报文伪造不了）—— pull 核来源
//! reply  往哪回（本端表里那一枚孔）            —— pull / push / close
//! probe  凭什么说它还在（**对端开的**那一枚）   —— probe
//! ```
//!
//! 探针为什么必须是"对端开的"：内核那条边是「开者退场 ⇒ 它开的资源一起封印」
//! （`kernel/src/work/unit/gate/cull.rs` 的 `seal_owned`，配 `doom` 的派生链），故只有
//! 对端开的那一枚会在对端退场时报 `Dead`/`Denied`——自己开的那一枚判不出对端死活。
//!
//! # 判活是两问
//!
//! ```text
//! probe   存在性：还在吗（只探不挂）
//! pull    响应性：到点没回、探过、还在 ⇒ Busy
//!                 到点没回、探过、没了 ⇒ Denied / Dead
//! ```
//!
//! **先等满，到点才探**：存在 ≠ 应答，反过来问就是拿"它还在"冒充"它会答"。
//!
//! # 推不动就是收场
//!
//! `push` 是**有界**推：到点仍推不进去 ⇒ 就地收场。无界推（`HolePie::push` 睡到有位置）
//! 配"单槽回信孔 + 服务只有一个请求线程"，一条没人排空的回信就能把整台服务钉死。

use env::{EnvError, EnvResult, HoleDir, PieToken, TaskId, make_err};
use runtime::env::chrono;
use runtime::env::mail::{self, AnyPie as _, HolePie};

/// D1 负码：无权 / 协议错（与各协议的负码同表）。
fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 单调时钟（纳秒）。`within` 在别处一律是毫秒，故截止也按纳秒算。
fn now_ns() -> EnvResult<u64> {
    let (secs, nanos) = chrono::clock()?;
    Ok(secs.saturating_mul(1_000_000_000).saturating_add(nanos))
}

/// 一条会话的三格。**开会话不在这里**：各协议自己开完，把三格交进来。
#[derive(Clone, Copy, Debug)]
pub struct Session {
    peer: TaskId,
    reply: PieToken,
    probe: PieToken,
}

impl Session {
    pub const fn new(peer: TaskId, reply: PieToken, probe: PieToken) -> Session {
        Session { peer, reply, probe }
    }

    /// 对端是谁：服务侧拿它核归属，客户端拿它核来源。
    pub const fn peer(self) -> TaskId {
        self.peer
    }

    /// 存在性：`Ok` = 还在；`Denied` = 已从表里摘掉；`Dead` = 已封印。
    ///
    /// 只探不挂（`millis = 0`）。方向位是签名的形状——存活判定在方向之前。
    pub fn probe(&self) -> EnvResult<()> {
        HolePie::from_token(self.probe)
            .wait(HoleDir::Pull, 0)
            .map(|_| ())
    }

    /// 响应性：有界收一条回信，并核来源 = 推者。
    ///
    /// - `Busy` —— 到点没回，**探过，它还在**（可重试）；
    /// - `Denied`/`Dead` —— 到点没回，**探过，它没了**；或这枚孔被别的推者污染了。
    pub fn pull<'a>(&self, buf: &'a mut [u8], within: usize) -> EnvResult<&'a [u8]> {
        let hole = HolePie::from_token(self.reply);
        match hole.pull_timeout_from(buf, within) {
            Ok((len, from)) if from == self.peer => buf.get(..len).ok_or_else(denied),
            // 来源不符：这枚孔被污染了（迟到的那条仍可能落槽），本会话不可再用。
            Ok(_) => {
                let _ = self.close();
                Err(denied())
            }
            Err(e) if e.source.is_busy() => match self.probe() {
                Ok(()) => Err(e),
                Err(gone) => {
                    let _ = self.close();
                    Err(gone)
                }
            },
            // -2 = Dead（`env::ecall` 的码表）：对端把那扇门封印了。
            Err(e) if e.source.code() == -2 => {
                let _ = self.close();
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    /// 有界推一条回信。任何失败都**就地收场**：推不动 = 这条会话没有出口了。
    pub fn push(&self, frame: &[u8], within: usize) -> EnvResult<()> {
        let hole = HolePie::from_token(self.reply);
        match push_within(&hole, frame, within) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = self.close();
                Err(e)
            }
        }
    }

    /// 收场：放掉回信孔的 pie。**幂等**——已经放过的（不在表里了）也返 `Ok`。
    ///
    /// 清格是持有者的事：服务侧那格是 `Option<Session>`，置空之后"已收场"不可表达。
    /// 探针**不在这里放**：它是对端开的那一枚，同一枚可能还是调用方手里的入口副本。
    pub fn close(&self) -> EnvResult<()> {
        let _ = HolePie::from_token(self.reply).release();
        Ok(())
    }
}

/// 有界推：槽满则等到 `within`，到点仍推不进去 ⇒ `Busy`（**不是**睡到有位置）。
///
/// `HolePie::push` 只有"睡到有位置"这一种策略（`wait(Push, usize::MAX)`），而链上每一跳
/// 都用它 ⇒ 一处不排空就把上游钉死，且没有上界可供诊断。本函数是那条上界的唯一出处。
pub fn push_within(hole: &HolePie, frame: &[u8], within: usize) -> EnvResult<()> {
    // 上界只在**第一次碰壁**时读钟：快路上一次 envcall 都不多花。
    let mut deadline = None;
    loop {
        match mail::push(hole.token(), frame.as_ptr(), frame.len()) {
            Ok(()) => return Ok(()),
            Err(e) if !e.source.is_busy() => return Err(e),
            Err(e) => {
                let now = now_ns()?;
                let end = *deadline
                    .get_or_insert_with(|| now.saturating_add(within as u64 * 1_000_000));
                if now >= end {
                    return Err(e);
                }
                let remain_ms = ((end - now) / 1_000_000).max(1) as usize;
                // 孔死了会当场报 `Dead`（`hole::wait` 先判存活），故这里不必自己看表。
                let _ = hole.wait(HoleDir::Push, remain_ms)?;
            }
        }
    }
}
