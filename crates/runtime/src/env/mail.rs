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

/// 解封 Void（**无数据面**的权柄载体）：造一枚只有身份与存活的许可载体。
///
/// **无参数**——没有 mtu、没有字节数。它承载**存在权**（"你能不能做某件事"），
/// 与资源权（"你对这份资源能做什么"）正交。
pub fn unseal_void() -> EnvResult<usize> {
    let r = PieCall::UnsealVoid.call()?;
    match r {
        PieCallRet::UnsealVoid(tk) => Ok(tk.get()),
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

/// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）。
///
/// 返回的是句柄而非裸数：它要经线形送到对方、再由对方 `from_token` 重建——
/// 全程一个 `PieToken`，中途不化成 `usize` 便不会与别的 id 混。
pub fn accord(src: PieToken, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
    let r = PieCall::Accord { src, dst, subset }.call()?;
    match r {
        PieCallRet::Accord(tk) => Ok(tk),
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

/// 收回我授给 `dst` 的副本（含其全部后代）。
///
/// `at_dst` = 该副本在**对端表里**的句柄（[`accord`] 的返回值，经线形送达）——
/// **不是我这边的 token**。鉴权 = 「这枚的 `sire` 在我表里」＝「它是我授出的」。
pub fn revoke(dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
    let r = PieCall::Revoke { dst, token: at_dst }.call()?;
    match r {
        PieCallRet::Revoke(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 收拢：本任务权限表第 `index` 份（token + permission + vestor）。
/// 越界 → `(0, 空权限, 0)`——哨兵不报错。
///
/// **唯一的枚举手段**：`handshake::moor()` 靠它发现「父域授给我的那枚门闩」。
/// 已知句柄求事实用 [`reserve`]；原始自持 pie（vestor = None）编码为 `TaskId(0)`，
/// 与 `UnitCall::SelfId` 越界哨兵一致。
pub fn collect(index: usize) -> EnvResult<(PieToken, env::Permission, TaskId)> {
    let r = PieCall::Collect { index }.call()?;
    match r {
        PieCallRet::Collect(r) => Ok(r),
        _ => unreachable!(),
    }
}

/// 查询：我持有的这枚门闩——`(vestor, owner)`。
///
/// `vestor` = 这枚门闩谁授的（转手即改写）；`owner` = 这扇门谁开的（副本共享同一
/// 事实）。求「对端是谁」一律用 `owner`：root 转发过的门闩，`vestor` 会变成 root。
pub fn reserve(token: PieToken) -> EnvResult<(TaskId, TaskId)> {
    let r = PieCall::Reserve { token }.call()?;
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

/// 权柄句柄 —— Hole 与 Pole 的**权柄操作同构**，故只写一遍。
///
/// 方法集 = `PieCall` 里「实例作用域 ∧ 与资源种类无关」那一类，一一对应，不多不少：
/// `Seal` / `Narrow` / `Accord` / `Revoke` / `Release`。
///
/// 不在本 trait 的，各有其理由：
/// - **构造**：`unseal*` 产出 `Self`，做不成 `&self` 方法
/// - **任务作用域**：`Collect`（按 index 枚举我表里的）、`Reserve`（按句柄查来历）
///   ——它们不作用在「某一个句柄」上
/// - **资源专属**：`Open`/`Shut`（Pole）、`Push`/`Pull`/`Wait`（Hole）
/// - **表示层转换**：`from_token` / `token` —— 与 ABI 无关
///
/// 镜像内核侧 `gate::AnyPie`（`enum { Hole, Pole }`，提供同一批跨种类方法）：
/// 同一条「权柄操作与资源种类无关」的知识在两侧各落一次，而不是散成四份。
pub trait AnyPie {
    /// 封印资源（**只有资源开辟者**可做）。
    ///
    /// 只置死并唤醒等待者，**不摘表项**——持有者仍须 [`release`](AnyPie::release)
    /// 收尾，否则表项泄漏。故 `release` 是唯一不过存活闸的操作。
    fn seal(&self) -> EnvResult<()>;

    /// 收窄本 pie 权限（就地改写，单调；`subset` ⊆ 当前权限）。
    ///
    /// Pole 多一条约束：`subset` 须含 READ（RISC-V PTE 无 R=0 的合法数据叶子），
    /// 且会同步把已映射段降权。Hole 无映射，故无此约束。
    fn narrow(&self, subset: env::Permission) -> EnvResult<()>;

    /// 转授子集给 `dst`，返回**对端侧**那枚的句柄（撤销句柄）——
    /// 对方用 `from_token(at_dst)` 重建。
    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken>;

    /// 收回我授给 `dst` 的副本（含其全部后代，幂等）。
    ///
    /// `at_dst` = 该副本在**对端表里**的句柄（[`accord`](AnyPie::accord) 的返回值，
    /// 经线形送达）——**不是我这边的 token**。鉴权 = 「这枚的 `sire` 在我表里」。
    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()>;

    /// 放下我这一份（含其全部后代；Pole 同步撤映射）。资源本身不动——封印用
    /// [`seal`](AnyPie::seal)。不需要任何权限位。
    fn release(&self) -> EnvResult<()>;
}

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
    pub fn from_token(token: impl Into<usize>) -> Self {
        Self {
            token: token.into(),
        }
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

    pub fn token(&self) -> usize {
        self.token
    }
}

impl AnyPie for VoidPie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
        accord(PieToken::new(self.token), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token)
    }
}

impl AnyPie for PolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
        accord(PieToken::new(self.token), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token)
    }
}

impl AnyPie for HolePie {
    fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    fn accord(&self, dst: TaskId, subset: env::Permission) -> EnvResult<PieToken> {
        accord(PieToken::new(self.token), dst, subset)
    }

    fn revoke(&self, dst: TaskId, at_dst: PieToken) -> EnvResult<()> {
        revoke(dst, at_dst)
    }

    fn release(&self) -> EnvResult<()> {
        release(self.token)
    }
}

/// Void 门闩用户态句柄——**三种句柄里唯一没有任何方法的那种**。
///
/// 它没有 `push`/`pull`（那是 Hole 的数据面）、没有 `open`/`shut`（那是 Pole 的
/// 页视图）。它能做的只有 [`AnyPie`] 那一套（`accord`/`narrow`/`revoke`/`release`
/// /`seal`）——因为它的全部内容就是"我持有这一枚"。
pub struct VoidPie {
    token: usize,
}

impl VoidPie {
    /// 解封一枚 Void（无参数：没有大小、没有对齐）。
    pub fn unseal() -> EnvResult<Self> {
        Ok(Self {
            token: unseal_void()?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: impl Into<usize>) -> Self {
        Self {
            token: token.into(),
        }
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
    pub fn from_token(token: impl Into<usize>) -> Self {
        Self {
            token: token.into(),
        }
    }

    pub fn open(&self) -> EnvResult<usize> {
        open(self.token)
    }

    pub fn shut(&self) -> EnvResult<()> {
        shut(self.token)
    }

    pub fn token(&self) -> usize {
        self.token
    }
}
