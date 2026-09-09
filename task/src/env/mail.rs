//! Mail 域：pie 门闩操作（Hole 数据过内核、Pole 页级安全内存）。
//!
//! 每个调用都是一次 envcall，由内核侧 dispatch 做 alive + rights + 分派。
//! 用户句柄统一为 per-pie `token`（全局唯一，u64）。
//!
//! 方案 3（typed payload）：`MailCall::X{ .. }.call()?` 直接得 `MailCallRet`，
//! 参数在构造时类型安全（PieToken/VirtAddr/TaskId/Permission），返回值经 from_pair
//! 蒸馏为 Ret 载荷。裸函数层只封 Ret、零业务逻辑。
//!
//! push/pull 的阻塞：内核 Push/Pull 槽满/槽空返 `-3 Busy`；本层转 `Wait` 原语
//! 挂起（让出 CPU），被对侧唤醒后重试——真阻塞，不占核。

use env::{EnvResult, HoleDir, MailCall, MailCallRet, PieToken, VirtAddr};

use crate::env::room;

/// hole 单消息字节上限（与内核侧 `HOLE_MTU_MAX` 一致）。调用方 unseal 时选
/// mtu ∈ [1, HOLE_MTU_MAX]；推送时实际字节数由 `push` 的 `len` 决定。
pub const HOLE_MTU_MAX: usize = 4096;

// ── 裸函数层（envcall 转发，零业务逻辑）──

/// 解封 Hole（mtu = 该孔单消息上限，1..=4096）。
pub fn unseal_hole(mtu: usize) -> EnvResult<u64> {
    let r = MailCall::UnsealHole { mtu }.call()?;
    match r {
        MailCallRet::UnsealHole(tk) => Ok(tk.get()),
        _ => unreachable!(),
    }
}

pub fn unseal_pole(bytes: usize) -> EnvResult<u64> {
    let r = MailCall::UnsealPole { bytes }.call()?;
    match r {
        MailCallRet::UnsealPole(tk) => Ok(tk.get()),
        _ => unreachable!(),
    }
}

/// push 一条消息（`msg[..len]` 进 hole 槽）。`len ∈ [1, 该孔 mtu]`。
pub fn push(token: u64, msg: *const u8, len: usize) -> EnvResult<()> {
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

/// pull 一条消息（最多装 `buf[..max]`）。返实际长度（≤ max）。
/// `max ≥ 1`，且 ≤该孔 mtu。
pub fn pull(token: u64, buf: *mut u8, max: usize) -> EnvResult<usize> {
    let r = MailCall::Pull {
        token: PieToken::new(token),
        buf: VirtAddr::new(buf as usize),
        max,
    }
    .call()?;
    match r {
        MailCallRet::Pull(n) => Ok(n),
        _ => unreachable!(),
    }
}

/// 等 hole 某方向就绪：`millis` 毫秒（`usize::MAX` = 永久，`0` = 只探测不挂起）。
/// 返回 `true` = 本次调用当场就绪；`false` = 未就绪（探测失败，或挂起过）。
pub fn wait(token: u64, dir: HoleDir, millis: usize) -> EnvResult<bool> {
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

pub fn map(token: u64) -> EnvResult<usize> {
    let r = MailCall::Map {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        MailCallRet::Map(va) => Ok(va.get()),
        _ => unreachable!(),
    }
}

pub fn unmap(token: u64) -> EnvResult<()> {
    let r = MailCall::Unmap {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        MailCallRet::Unmap(()) => Ok(()),
        _ => unreachable!(),
    }
}

pub fn seal(token: u64) -> EnvResult<()> {
    let r = MailCall::Seal {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        MailCallRet::Seal(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 转授子集给其他 Task：src_token + dst_id + subset → 新 pie 的 token（撤销句柄）。
pub fn accord(src_token: u64, dst_id: usize, subset: env::Permission) -> EnvResult<u64> {
    let r = MailCall::Accord {
        src: PieToken::new(src_token),
        dst: env::TaskId::new(dst_id),
        subset,
    }
    .call()?;
    match r {
        MailCallRet::Accord(tk) => Ok(tk.get()),
        _ => unreachable!(),
    }
}

/// 收窄本 pie 权限（就地改写；Pole 同步降页表）。
pub fn narrow(token: u64, subset: env::Permission) -> EnvResult<()> {
    let r = MailCall::Narrow {
        token: PieToken::new(token),
        subset,
    }
    .call()?;
    match r {
        MailCallRet::Narrow(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 收回授与他人的副本：dst_id + token。
pub fn revoke(dst_id: usize, token: u64) -> EnvResult<()> {
    let r = MailCall::Revoke {
        dst: env::TaskId::new(dst_id),
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        MailCallRet::Revoke(()) => Ok(()),
        _ => unreachable!(),
    }
}

/// 收拢：本任务权限表第 `index` 份（token + permission）。越界 → `(0, 空权限)`。
pub fn collect(index: usize) -> EnvResult<(u64, env::Permission)> {
    let r = MailCall::Collect { index }.call()?;
    match r {
        MailCallRet::Collect((token, permission)) => Ok((token.get(), permission)),
        _ => unreachable!(),
    }
}

/// 放下：自释本任务的一份门闩（Pole 同步 unmap）。表里无此 token → -1。
pub fn release(token: u64) -> EnvResult<()> {
    let r = MailCall::Release {
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        MailCallRet::Release(()) => Ok(()),
        _ => unreachable!(),
    }
}

// ── 类型化句柄（编译期区分 Hole / Pole）──

/// Hole 门闩用户态句柄。
pub struct HolePie {
    token: u64,
}

impl HolePie {
    /// 解封 Hole（mtu = 该孔单消息上限，1..=4096）。
    pub fn unseal(mtu: usize) -> EnvResult<Self> {
        Ok(Self {
            token: unseal_hole(mtu)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: u64) -> Self {
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

    pub fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    /// 收窄本 pie 权限（就地改写，单调；subset ⊆ 当前权限）。
    pub fn narrow(&self, subset: env::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    /// 转授子集给 dst_task。subset ⊆ self.permission。
    /// 返回新 pie 的 token（撤销句柄）——对方用 `HolePie::from_token(token)` 重建。
    pub fn accord(&self, dst_id: usize, subset: env::Permission) -> EnvResult<u64> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: u64) -> EnvResult<()> {
        revoke(dst_id, token)
    }

    /// 放下我这一份（自释；资源本身不动——封印用 `seal`）。
    pub fn release(&self) -> EnvResult<()> {
        release(self.token)
    }

    pub fn token(&self) -> u64 {
        self.token
    }
}

/// Pole 门闩用户态句柄。
pub struct PolePie {
    token: u64,
}

impl PolePie {
    pub fn unseal(bytes: usize) -> EnvResult<Self> {
        Ok(Self {
            token: unseal_pole(bytes)?,
        })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: u64) -> Self {
        Self { token }
    }

    pub fn map(&self) -> EnvResult<usize> {
        map(self.token)
    }

    pub fn unmap(&self) -> EnvResult<()> {
        unmap(self.token)
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
    pub fn accord(&self, dst_id: usize, subset: env::Permission) -> EnvResult<u64> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: u64) -> EnvResult<()> {
        revoke(dst_id, token)
    }

    /// 放下我这一份（自释；资源本身不动——封印用 `seal`）。
    pub fn release(&self) -> EnvResult<()> {
        release(self.token)
    }

    pub fn token(&self) -> u64 {
        self.token
    }
}
