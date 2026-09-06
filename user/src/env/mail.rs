//! Mail 域：pie 门闩操作（Hole 数据过内核、Pole 页级安全内存）。
//!
//! 每个调用都是一次 envcall，由内核侧 dispatch 做 alive + rights + 分派。
//! 用户句柄统一为 per-pie `token`（全局唯一，u64）。
//!
//! 方案 3（typed payload）：`MailCall::X{ .. }.call()?` 直接得 `MailCallRet`，
//! 参数在构造时类型安全（PieToken/VirtAddr/TaskId/Permission），返回值经 from_pair
//! 蒸馏为 Ret 载荷。裸函数层只封 Ret、零业务逻辑。

use ubi::{MailCall, MailCallRet, PieToken, EnvResult, VirtAddr};

/// Hole 单消息字节数（与内核侧 `HOLE_MSG_LEN` 一致）。
pub const HOLE_MSG_LEN: usize = 64;

// ── 裸函数层（envcall 转发，零业务逻辑）──

pub fn unseal_hole() -> EnvResult<u64> {
    let r = MailCall::UnsealHole.call()?;
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

pub fn push(token: u64, msg: *const [u8; HOLE_MSG_LEN]) -> EnvResult<()> {
    let r = MailCall::Push {
        token: PieToken::new(token),
        msg: VirtAddr::new(msg as usize),
    }
    .call()?;
    match r {
        MailCallRet::Push(()) => Ok(()),
        _ => unreachable!(),
    }
}

pub fn pull(token: u64, buf: *mut [u8; HOLE_MSG_LEN]) -> EnvResult<()> {
    let r = MailCall::Pull {
        token: PieToken::new(token),
        buf: VirtAddr::new(buf as usize),
    }
    .call()?;
    match r {
        MailCallRet::Pull(()) => Ok(()),
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
pub fn accord(src_token: u64, dst_id: usize, subset: ubi::Permission) -> EnvResult<u64> {
    let r = MailCall::Accord {
        src: PieToken::new(src_token),
        dst: ubi::TaskId::new(dst_id),
        subset,
    }
    .call()?;
    match r {
        MailCallRet::Accord(tk) => Ok(tk.get()),
        _ => unreachable!(),
    }
}

/// 收窄本 pie 权限（就地改写；Pole 同步降页表）。
pub fn narrow(token: u64, subset: ubi::Permission) -> EnvResult<()> {
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
        dst: ubi::TaskId::new(dst_id),
        token: PieToken::new(token),
    }
    .call()?;
    match r {
        MailCallRet::Revoke(()) => Ok(()),
        _ => unreachable!(),
    }
}

// ── 类型化句柄（编译期区分 Hole / Pole）──

/// Hole 门闩用户态句柄。
pub struct HolePie {
    token: u64,
}

impl HolePie {
    pub fn unseal() -> EnvResult<Self> {
        Ok(Self { token: unseal_hole()? })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: u64) -> Self {
        Self { token }
    }

    pub fn push(&self, msg: &[u8; HOLE_MSG_LEN]) -> EnvResult<()> {
        push(self.token, msg as *const [u8; HOLE_MSG_LEN])
    }

    pub fn pull(&self, buf: &mut [u8; HOLE_MSG_LEN]) -> EnvResult<()> {
        pull(self.token, buf as *mut [u8; HOLE_MSG_LEN])
    }

    pub fn seal(&self) -> EnvResult<()> {
        seal(self.token)
    }

    /// 收窄本 pie 权限（就地改写，单调；subset ⊆ 当前权限）。
    pub fn narrow(&self, subset: ubi::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    /// 转授子集给 dst_task。subset ⊆ self.permission。
    /// 返回新 pie 的 token（撤销句柄）——对方用 `HolePie::from_token(token)` 重建。
    pub fn accord(&self, dst_id: usize, subset: ubi::Permission) -> EnvResult<u64> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: u64) -> EnvResult<()> {
        revoke(dst_id, token)
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
        Ok(Self { token: unseal_pole(bytes)? })
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
    pub fn narrow(&self, subset: ubi::Permission) -> EnvResult<()> {
        narrow(self.token, subset)
    }

    /// 转授子集给 dst_task。subset ⊆ self.permission。
    /// 返回新 pie 的 token（撤销句柄）——对方用 `PolePie::from_token(token)` 重建。
    pub fn accord(&self, dst_id: usize, subset: ubi::Permission) -> EnvResult<u64> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: u64) -> EnvResult<()> {
        revoke(dst_id, token)
    }

    pub fn token(&self) -> u64 {
        self.token
    }
}
