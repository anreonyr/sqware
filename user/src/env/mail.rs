//! Mail 域：pie 门闩操作（Hole 数据过内核、Pole 页级安全内存）。
//!
//! 每个调用都是一次 envcall，由内核侧 dispatch 做 alive + rights + 分派。
//! 用户句柄统一为 per-pie `token`（全局唯一，u64）。

use ubi::{MailCall, UArgs, UResult, Ucall, UcallBuilder};

/// Hole 单消息字节数（与内核侧 `HOLE_MSG_LEN` 一致）。
pub const HOLE_MSG_LEN: usize = 64;

// ── 裸函数层（envcall 转发，零业务逻辑）──

pub fn unseal_hole() -> UResult<u64> {
    let (token, _) = UcallBuilder::new(Ucall::Mail(MailCall::UnsealHole)).call()?;
    Ok(token as u64)
}

pub fn unseal_pole(bytes: usize) -> UResult<u64> {
    let args = UArgs {
        a0: bytes,
        ..UArgs::default()
    };
    let (token, _) = UcallBuilder::new(Ucall::Mail(MailCall::UnsealPole))
        .args(args)
        .call()?;
    Ok(token as u64)
}

pub fn push(token: u64, msg: *const [u8; HOLE_MSG_LEN]) -> UResult<()> {
    let args = UArgs {
        a0: token as usize,
        a1: msg as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Push))
        .args(args)
        .call()?;
    Ok(())
}

pub fn pull(token: u64, buf: *mut [u8; HOLE_MSG_LEN]) -> UResult<()> {
    let args = UArgs {
        a0: token as usize,
        a1: buf as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Pull))
        .args(args)
        .call()?;
    Ok(())
}

pub fn map(token: u64) -> UResult<usize> {
    let args = UArgs {
        a0: token as usize,
        ..UArgs::default()
    };
    let (va, _) = UcallBuilder::new(Ucall::Mail(MailCall::Map))
        .args(args)
        .call()?;
    Ok(va)
}

pub fn unmap(token: u64) -> UResult<()> {
    let args = UArgs {
        a0: token as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Unmap))
        .args(args)
        .call()?;
    Ok(())
}

pub fn seal(token: u64) -> UResult<()> {
    let args = UArgs {
        a0: token as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Seal))
        .args(args)
        .call()?;
    Ok(())
}

/// 转授子集给其他 Task：a0 = src_token, a1 = dst_id, a2 = subset bits。
/// 返回新 pie 的 token（撤销句柄）。
pub fn accord(src_token: u64, dst_id: usize, subset: ubi::Permission) -> UResult<u64> {
    let args = UArgs {
        a0: src_token as usize,
        a1: dst_id,
        a2: subset.bits() as usize,
        ..UArgs::default()
    };
    let (token, _) = UcallBuilder::new(Ucall::Mail(MailCall::Accord))
        .args(args)
        .call()?;
    Ok(token as u64)
}

/// 收窄本 pie 权限（就地改写；Pole 同步降页表）。
pub fn narrow(token: u64, subset: ubi::Permission) -> UResult<()> {
    let args = UArgs {
        a0: token as usize,
        a1: subset.bits() as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Narrow))
        .args(args)
        .call()?;
    Ok(())
}

/// 收回授与他人的副本：a0 = dst_id, a1 = token。
pub fn revoke(dst_id: usize, token: u64) -> UResult<()> {
    let args = UArgs {
        a0: dst_id,
        a1: token as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Revoke))
        .args(args)
        .call()?;
    Ok(())
}

// ── 类型化句柄（编译期区分 Hole / Pole）──

/// Hole 门闩用户态句柄。
pub struct HolePie {
    token: u64,
}

impl HolePie {
    pub fn unseal() -> UResult<Self> {
        Ok(Self { token: unseal_hole()? })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: u64) -> Self {
        Self { token }
    }

    pub fn push(&self, msg: &[u8; HOLE_MSG_LEN]) -> UResult<()> {
        push(self.token, msg as *const [u8; HOLE_MSG_LEN])
    }

    pub fn pull(&self, buf: &mut [u8; HOLE_MSG_LEN]) -> UResult<()> {
        pull(self.token, buf as *mut [u8; HOLE_MSG_LEN])
    }

    pub fn seal(&self) -> UResult<()> {
        seal(self.token)
    }

    /// 收窄本 pie 权限（就地改写，单调；subset ⊆ 当前权限）。
    pub fn narrow(&self, subset: ubi::Permission) -> UResult<()> {
        narrow(self.token, subset)
    }

    /// 转授子集给 dst_task。subset ⊆ self.permission。
    /// 返回新 pie 的 token（撤销句柄）——对方用 `HolePie::from_token(token)` 重建。
    pub fn accord(&self, dst_id: usize, subset: ubi::Permission) -> UResult<u64> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: u64) -> UResult<()> {
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
    pub fn unseal(bytes: usize) -> UResult<Self> {
        Ok(Self { token: unseal_pole(bytes)? })
    }

    /// 由 token 重建句柄（用于接收 accord 来的 pie）。
    pub fn from_token(token: u64) -> Self {
        Self { token }
    }

    pub fn map(&self) -> UResult<usize> {
        map(self.token)
    }

    pub fn unmap(&self) -> UResult<()> {
        unmap(self.token)
    }

    pub fn seal(&self) -> UResult<()> {
        seal(self.token)
    }

    /// 收窄本 pie 权限（就地改写，单调；subset ⊆ 当前权限且须含 READ——RISC-V
    /// PTE 无 R=0 合法数据叶子）。Pole 会同步把映射段降权。
    pub fn narrow(&self, subset: ubi::Permission) -> UResult<()> {
        narrow(self.token, subset)
    }

    /// 转授子集给 dst_task。subset ⊆ self.permission。
    /// 返回新 pie 的 token（撤销句柄）——对方用 `PolePie::from_token(token)` 重建。
    pub fn accord(&self, dst_id: usize, subset: ubi::Permission) -> UResult<u64> {
        accord(self.token, dst_id, subset)
    }

    /// 收回授与 dst_id 的、由 token 标识的副本（须是本 pie accord 出的）。
    pub fn revoke(&self, dst_id: usize, token: u64) -> UResult<()> {
        revoke(dst_id, token)
    }

    pub fn token(&self) -> u64 {
        self.token
    }
}
