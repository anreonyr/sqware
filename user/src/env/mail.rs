//! Mail 域：pie 门闩操作（Hole 数据过内核、Pole 页级安全内存）。
//!
//! 每个调用都是一次 envcall，由内核侧 dispatch 做 alive + rights + 分派。

use ubi::{MailCall, UArgs, UResult, Ucall, UcallBuilder};

/// Hole 单消息字节数（与内核侧 `HOLE_MSG_LEN` 一致）。
pub const HOLE_MSG_LEN: usize = 64;

// ── Hole 门闩 ──

pub fn hole_open() -> UResult<usize> {
    let (idx, _) = UcallBuilder::new(Ucall::Mail(MailCall::OpenHole)).call()?;
    Ok(idx)
}

pub fn pole_open(bytes: usize) -> UResult<usize> {
    let args = UArgs {
        a0: bytes,
        ..UArgs::default()
    };
    let (idx, _) = UcallBuilder::new(Ucall::Mail(MailCall::OpenPole))
        .args(args)
        .call()?;
    Ok(idx)
}

pub fn hole_push(idx: usize, msg: *const [u8; HOLE_MSG_LEN]) -> UResult<()> {
    let args = UArgs {
        a0: idx,
        a1: msg as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Push))
        .args(args)
        .call()?;
    Ok(())
}

pub fn hole_pull(idx: usize, buf: *mut [u8; HOLE_MSG_LEN]) -> UResult<()> {
    let args = UArgs {
        a0: idx,
        a1: buf as usize,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Pull))
        .args(args)
        .call()?;
    Ok(())
}

pub fn pole_map(idx: usize) -> UResult<usize> {
    let args = UArgs {
        a0: idx,
        ..UArgs::default()
    };
    let (va, _) = UcallBuilder::new(Ucall::Mail(MailCall::Map))
        .args(args)
        .call()?;
    Ok(va)
}

pub fn pole_unmap(idx: usize) -> UResult<()> {
    let args = UArgs {
        a0: idx,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Unmap))
        .args(args)
        .call()?;
    Ok(())
}

pub fn shut(idx: usize) -> UResult<()> {
    let args = UArgs {
        a0: idx,
        ..UArgs::default()
    };
    UcallBuilder::new(Ucall::Mail(MailCall::Shut))
        .args(args)
        .call()?;
    Ok(())
}

/// 派门闩（Vest）：把 pie 复制给其他 Task。
/// a0 = src_pie_idx, a1 = target_task_id, a2 = subset bits → a0 = 新 pie_idx
pub fn vest(src_idx: usize, target_task_id: usize, subset: ubi::Permission) -> UResult<usize> {
    let args = UArgs {
        a0: src_idx,
        a1: target_task_id,
        a2: subset.bits() as usize,
        ..UArgs::default()
    };
    let (idx, _) = UcallBuilder::new(Ucall::Mail(MailCall::Vest))
        .args(args)
        .call()?;
    Ok(idx)
}

// ── 类型化句柄（编译期区分 Hole / Pole）──

/// Hole 门闩用户态句柄。
pub struct HolePie {
    idx: usize,
}

impl HolePie {
    pub fn open() -> UResult<Self> {
        Ok(Self { idx: hole_open()? })
    }

    /// 由 pie_idx 重建句柄（用于接收 Vest 来的 pie）。
    pub fn from_idx(idx: usize) -> Self {
        Self { idx }
    }

    pub fn push(&self, msg: &[u8; HOLE_MSG_LEN]) -> UResult<()> {
        hole_push(self.idx, msg as *const [u8; HOLE_MSG_LEN])
    }

    pub fn pull(&self, buf: &mut [u8; HOLE_MSG_LEN]) -> UResult<()> {
        hole_pull(self.idx, buf as *mut [u8; HOLE_MSG_LEN])
    }

    pub fn shut(&self) -> UResult<()> {
        shut(self.idx)
    }

    /// 把本 pie 复制给 target_task。subset ⊆ self.permission。
    /// 返回 target.pies 里新 pie 的 idx——对方用 `HolePie::from_idx(idx)` 重建句柄。
    pub fn vest(
        &self,
        target_task_id: usize,
        subset: ubi::Permission,
    ) -> UResult<usize> {
        vest(self.idx, target_task_id, subset)
    }

    pub fn idx(&self) -> usize {
        self.idx
    }
}

/// Pole 门闩用户态句柄。
pub struct PolePie {
    idx: usize,
}

impl PolePie {
    pub fn open(bytes: usize) -> UResult<Self> {
        Ok(Self { idx: pole_open(bytes)? })
    }

    /// 由 pie_idx 重建句柄（用于接收 Vest 来的 pie）。
    pub fn from_idx(idx: usize) -> Self {
        Self { idx }
    }

    pub fn map(&self) -> UResult<usize> {
        pole_map(self.idx)
    }

    pub fn unmap(&self) -> UResult<()> {
        pole_unmap(self.idx)
    }

    pub fn shut(&self) -> UResult<()> {
        shut(self.idx)
    }

    /// 把本 pie 复制给 target_task。subset ⊆ self.permission。
    pub fn vest(
        &self,
        target_task_id: usize,
        subset: ubi::Permission,
    ) -> UResult<usize> {
        vest(self.idx, target_task_id, subset)
    }

    pub fn idx(&self) -> usize {
        self.idx
    }
}