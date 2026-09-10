//! Mail 域：门闩操作。
//!
//! **两条轴**：权柄（`PieCall`，class 7）走 `unseal_*`/`open`/`shut`/`seal`/`accord`
//! /`narrow`/`revoke`/`collect`/`reserve`/`release`；数据（`MailCall`，class 5）走
//! `push`/`pull`/`wait`。本模块是两者的**裸函数层**——每个函数封一次 envcall，
//! 零业务逻辑。
//!
//! 每个调用都是一次 envcall，由内核侧 dispatch 做 alive + rights + 分派。
//! 用户句柄统一为 per-pie `token`（全局唯一，usize）。
//!
//! 方案 3（typed payload）：`PieCall::X{ .. }.call()?` 直接得 `PieCallRet`，
//! 参数在构造时类型安全（PieToken/VirtAddr/TaskId/Permission），返回值经 from_pair
//! 蒸馏为 Ret 载荷。裸函数层只封 Ret、零业务逻辑。
//!
//! push/pull 的阻塞：内核 Push/Pull 槽满/槽空返 `-3 Busy`；本层转 `Wait` 原语
//! 挂起（让出 CPU），被对侧唤醒后重试——真阻塞，不占核。

use env::{
    EnvResult, HoleDir, MailCall, MailCallRet, PieCall, PieCallRet, PieToken, TaskId, VirtAddr,
};

/// hole 单消息字节上限（与内核侧 `HOLE_MTU_MAX` 一致）。调用方 unseal 时选
/// mtu ∈ [1, HOLE_MTU_MAX]；推送时实际字节数由 `push` 的 `len` 决定。
pub const HOLE_MTU_MAX: usize = 4096;

/// 单调时钟读数（纳秒）——`pull_timeout` 的 deadline 用（机器无关，不依赖
/// timebase 频率）。
fn now_ns() -> EnvResult<u64> {
    let (secs, nanos) = crate::env::chrono::clock()?;
    Ok(secs.saturating_mul(1_000_000_000).saturating_add(nanos))
}

// ── 裸函数层（envcall 转发，零业务逻辑）──

/// 解封 Hole（mtu = 该孔单消息上限，1..=4096）。
pub fn unseal_hole(mtu: usize) -> EnvResult<usize> {
    let r = PieCall::UnsealHole { mtu }.call()?;
    match r {
        PieCallRet::UnsealHole(tk) => Ok(tk.get()),
        _ => unreachable!(),
    }
}

pub fn unseal_pole(bytes: usize) -> EnvResult<usize> {
    let r = PieCall::UnsealPole { bytes }.call()?;
    match r {
        PieCallRet::UnsealPole(tk) => Ok(tk.get()),
        _ => unreachable!(),
    }
}

/// push 一条消息（`msg[..len]` 进 hole 槽）。`len ∈ [1, 该孔 mtu]`。
pub fn push(token: usize, msg: *const u8, len: usize) -> EnvResult<()> {
    let r = MailCall::Push {
        token: PieToken::new(token),
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
/// `max ≥ 1`，且 ≤该孔 mtu。
pub fn pull(token: usize, buf: *mut u8, max: usize) -> EnvResult<usize> {
    pull_from(token, buf, max).map(|(n, _)| n)
}

/// pull 一条消息并取回**发送者**（`(长度, 发送者 TaskId)`）。
///
/// 发送者由内核在 `Push` 时盖章——身份不可伪造，不必再从报文里猜。
pub fn pull_from(token: usize, buf: *mut u8, max: usize) -> EnvResult<(usize, TaskId)> {
    let r = MailCall::Pull {
        token: PieToken::new(token),
        buf: VirtAddr::new(buf as usize),
        max,
    }
    .call()?;
    match r {
        MailCallRet::Pull((n, from)) => Ok((n, from)),
        _ => unreachable!(),
    }
}

/// 等 hole 某方向就绪：`millis` 毫秒（`usize::MAX` = 永久，`0` = 只探测不挂起）。
/// 返回 `true` = 本次调用当场就绪；`false` = 未就绪（探测失败，或挂起过）。
pub fn wait(token: usize, dir: HoleDir, millis: usize) -> EnvResult<bool> {
    let r = MailCall::Wait {
        token: PieToken::new(token),
        dir,
        millis,
    }
    .call()?;
    match r {
        MailCallRet::Wait(ready) => Ok(ready),
        _ => unreachable!(),
    }
}

pub fn open(token: usize) -> EnvResult<usize> {
    let r = PieCall::Open {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        PieCallRet::Open(va) => Ok(va.get()),
        _ => unreachable!(),
    }
}

pub fn shut(token: usize) -> EnvResult<()> {
    let r = PieCall::Shut {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        PieCallRet::Shut(()) => Ok(()),
        _ => unreachable!(),
    }
}

pub fn seal(token: usize) -> EnvResult<()> {
    let r = PieCall::Seal {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        PieCallRet::Seal(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 转授子集给其他 Task：src_token + dst_id + subset → 新 pie 的 token（撤销句柄）。
pub fn accord(src_token: usize, dst_id: usize, subset: env::Permission) -> EnvResult<usize> {
    let r = PieCall::Accord {
        src: PieToken::new(src_token),
        dst: env::TaskId::new(dst_id),
        subset,
    }
    .call()?;
    match r {
        PieCallRet::Accord(tk) => Ok(tk.get()),
        _ => unreachable!(),
    }
}

/// 收窄本 pie 权限（就地改写；Pole 同步降页表）。
pub fn narrow(token: usize, subset: env::Permission) -> EnvResult<()> {
    let r = PieCall::Narrow {
        token: PieToken::new(token),
        subset,
    }
    .call()?;
    match r {
        PieCallRet::Narrow(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 收回授与他人的副本：dst_id + token。
pub fn revoke(dst_id: usize, token: usize) -> EnvResult<()> {
    let r = PieCall::Revoke {
        dst: env::TaskId::new(dst_id),
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        PieCallRet::Revoke(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 收拢：本任务权限表第 `index` 份（token + permission + vestor）。
/// 越界 → `(0, 空权限, 0)`——哨兵不报错。
///
/// **唯一的枚举手段**：`handshake::moor()` 靠它发现「父域授给我的那枚门闩」。
/// 已知句柄求事实用 [`owned`]；原始自持 pie（vestor = None）编码为 `TaskId(0)`，
/// 与 `UnitCall::SelfId` 越界哨兵一致。
pub fn collect(index: usize) -> EnvResult<(usize, env::Permission, env::TaskId)> {
    let r = PieCall::Collect { index }.call()?;
    match r {
        PieCallRet::Collect((token, permission, vestor)) => Ok((token.get(), permission, vestor)),
        _ => unreachable!(),
    }
}

/// 查询：我持有的这枚门闩——`(vestor, owner)`。
///
/// `vestor` = 这枚门闩谁授的（转手即改写）；`owner` = 这扇门谁开的（副本共享同一
/// 事实）。求「对端是谁」一律用 `owner`：root 转发过的门闩，`vestor` 会变成 root。
pub fn owned(token: usize) -> EnvResult<(env::TaskId, env::TaskId)> {
    let r = PieCall::Reserve {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        PieCallRet::Reserve((vestor, owner)) => Ok((vestor, owner)),
        _ => unreachable!(),
    }
}

/// 放下：自释本任务的一份门闩（Pole 同步 unmap）。表里无此 token → -1。
pub fn release(token: usize) -> EnvResult<()> {
    let r = PieCall::Release {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        PieCallRet::Release(()) => Ok(()),
        _ => unreachable!(),
    }
}

// ── 类型化句柄（编译期区分 Hole / Pole）──

/// Hole 门闩用户态句柄。
pub struct HolePie {
    token: usize,
}

impl HolePie {
    /// 解封 Hole（mtu = 该孔单消息上限，1..=4096）。
    pub fn unseal(mtu: usize) -> EnvResult<Self> {
        Ok(Self {
            token: unseal_hole(mtu)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: usize) -> Self {
        Self { token }
    }

    /// 等某方向就绪：`millis` 毫秒（`usize::MAX` = 永久，`0` = 只探测不挂起）。
    /// 返回 `true` = 调用当场就绪；`false` = 未就绪（探测失败，或挂起过）。
    pub fn wait(&self, dir: HoleDir, millis: usize) -> EnvResult<bool> {
        wait(self.token, dir, millis)
    }

    /// 写消息（任意长度 ≤ mtu）：槽满则睡到有空间（让出 CPU）。
    pub fn push(&self, msg: &[u8]) -> EnvResult<()> {
        loop {
            match push(self.token, msg.as_ptr(), msg.len()) {
                Ok(()) => return Ok(()),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Push, usize::MAX)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 取消息：槽空则睡到有信（让出 CPU）。返实际收到字节数（≤ `buf.len()`）。
    pub fn pull(&self, buf: &mut [u8]) -> EnvResult<usize> {
        loop {
            match pull(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(n) => return Ok(n),
                Err(e) if e.source.is_busy() => {
                    self.wait(HoleDir::Pull, usize::MAX)?;
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
                    self.wait(HoleDir::Pull, usize::MAX)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 有界 pull：槽空则最多等 `millis` 毫秒；仍无消息 → `Err(Busy)`（码 -3）。
    ///
    /// 用于「等对端回复」这类必须有上界的往返：无限等会把协议错误（回复被丢弃、
    /// 对端漏回）变成不可诊断的挂起。**超时后该 hole 不再"干净"**——迟到的回复
    /// 仍可能落进槽里，使下一次 pull 取到上一条；调用方应弃用该会话。
    ///
    /// 实现要点：`wait` 返 false **不等于**超时——它可能是「唤醒闩（pend）被消费」
    /// 或一次无关唤醒（见 `messenger::wake`：无等待者时置 pend，而成功裸 pull 不会
    /// 消费它，故 pend 可能是陈旧的）。所以这里按 **deadline 循环**：只有 `clock()`
    /// 真的走完 `millis` 才报 Busy，否则带着剩余时间重试。
    pub fn pull_timeout(&self, buf: &mut [u8], millis: usize) -> EnvResult<usize> {
        let deadline = now_ns()?.saturating_add((millis as u64).saturating_mul(1_000_000));
        loop {
            match pull(self.token, buf.as_mut_ptr(), buf.len()) {
                Ok(n) => return Ok(n),
                Err(e) if e.source.is_busy() => {
                    let now = now_ns()?;
                    if now >= deadline {
                        return pull(self.token, buf.as_mut_ptr(), buf.len());
                    }
                    let remain_ms = ((deadline - now) / 1_000_000).max(1) as usize;
                    let _ = self.wait(HoleDir::Pull, remain_ms)?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    /// 收窄本 pie 权限（就地改写，单调；subset ⊆ 当前权限）。
    pub fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    /// 转授子集给 dst_task。subset ⊆ self.permission。
    /// 返回新 pie 的 token（撤销句柄）——对方用 `HolePie::from_token(token)` 重建。
    pub fn accord(&self, dst_id: usize, subset: env::Permission) -> EnvResult<usize> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: usize) -> EnvResult<()> {
        revoke(dst_id, token)
    }

    /// 放下我这一份（自释；资源本身不动——封印用 `seal`）。
    pub fn release(&self) -> EnvResult<()> {
        release(self.token)
    }

    pub fn token(&self) -> usize {
        self.token
    }
}

/// Pole 门闩用户态句柄。
pub struct PolePie {
    token: usize,
}

impl PolePie {
    pub fn unseal(bytes: usize) -> EnvResult<Self> {
        Ok(Self {
            token: unseal_pole(bytes)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: usize) -> Self {
        Self { token }
    }

    pub fn open(&self) -> EnvResult<usize> {
        open(self.token)
    }

    pub fn shut(&self) -> EnvResult<()> {
        shut(self.token)
    }

    pub fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    /// 收窄本 pie 权限（就地改写，单调；subset ⊆ 当前权限且须含 READ——RISC-V
    /// PTE 无 R=0 合法数据叶子）。Pole 会同步把映射段降权。
    pub fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    /// 转授子集给 dst_task。subset ⊆ self.permission。
    /// 返回新 pie 的 token（撤销句柄）——对方用 `PolePie::from_token(token)` 重建。
    pub fn accord(&self, dst_id: usize, subset: env::Permission) -> EnvResult<usize> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: usize) -> EnvResult<()> {
        revoke(dst_id, token)
    }

    /// 放下我这一份（自释；资源本身不动——封印用 `seal`）。
    pub fn release(&self) -> EnvResult<()> {
        release(self.token)
    }

    pub fn token(&self) -> usize {
        self.token
    }
}
