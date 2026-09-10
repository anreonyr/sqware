//! envcall·wire — 环境调用载荷的字段 ↔ usize 契约（pack 与校验式 unpack）。
//!
//! 本文件 = **契约核心**：[`Wire`] trait、[`Decode`] 失败域、基元与权限位的 impl。
//! 字段**词汇表**按语义分居三个子模块（本文件 re-export，故 `env::wire::X` 路径不变）：
//!   - [`handle`] —— 语义句柄（PieToken / TaskId / TeamId / VirtAddr）；
//!   - [`frompair`] —— 内核回写的 `(a0, a1)` → 域 Ret 载荷蒸馏；
//!   - [`name`] —— 定长名字（[`NAME_LEN`] / [`Name`] / [`NameError`]）。
//!
//! 这是方案 3（typed payload）的**唯一类型擦除点**：每个字段类型都实现 [`Wire`]，
//! 由 [`derive(Envcall)`](envmacros) 生成的 codec 自动接线，用户侧与内核侧不再手写
//! `as usize` / `from_bits_truncate`。非法位校验收敛在此：`Permission`/`PteFlags`
//! 的 unpack 是 `from_bits(...).ok_or(...)`，而非静默截断。
//!
//! 通用性：本 trait 只依赖 `usize`，不绑 U-mode 语义——sbi 等 S-mode 调用封装
//! 未来可直接复用同一 codec（derive 不写死 envcall 路径）。

pub mod frompair;
pub mod handle;
pub mod name;

pub use frompair::FromPair;
pub use handle::{PieToken, TaskId, TeamId, VirtAddr};
pub use name::{NAME_LEN, Name, NameError};

/// 字段 ↔ usize 的契约。
pub trait Wire: Sized {
    /// 把自身 pack 进 `s`，游标 `i` 前进一格。
    fn pack(&self, s: &mut [usize; 6], i: &mut usize);
    /// 从 `s` 读出自身，游标 `i` 前进一格；非法位 → `Err(Decode)`。
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode>;
}

/// 解码错误：unpack 失败（含非法位校验拒绝）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decode {
    /// 未知调用号（index 越界）。
    BadSlot,
    /// 字段数超出 a0..a5。
    Overflow,
    /// 非法位（如 `Permission`/`PteFlags` 含未定义位，或 bool 非 0/1）。
    Invalid,
}

// ── 基元 ────────────────────────────────────────────────────────────────

impl Wire for usize {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = *self;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(v)
    }
}

impl Wire for u64 {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = *self as usize;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)? as u64;
        *i += 1;
        Ok(v)
    }
}

impl Wire for bool {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = *self as usize;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        match v {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Decode::Invalid),
        }
    }
}

impl Wire for crate::fid::HoleDir {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = match self {
            crate::fid::HoleDir::Pull => 0,
            crate::fid::HoleDir::Push => 1,
        };
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        match v {
            0 => Ok(crate::fid::HoleDir::Pull),
            1 => Ok(crate::fid::HoleDir::Push),
            _ => Err(Decode::Invalid),
        }
    }
}

impl Wire for crate::fid::ProgramKind {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = match self {
            crate::fid::ProgramKind::User => 0,
            crate::fid::ProgramKind::Supervisor => 1,
        };
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        match v {
            0 => Ok(crate::fid::ProgramKind::User),
            1 => Ok(crate::fid::ProgramKind::Supervisor),
            _ => Err(Decode::Invalid),
        }
    }
}

/// 权限位掩码（envcall 单一真相；unpack 走 `from_bits` 校验，非法位 → `Invalid`，
/// 替代旧 `from_bits_truncate` 的静默截断）。
impl Wire for crate::permission::Permission {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.bits() as usize;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)? as u32;
        *i += 1;
        crate::permission::Permission::from_bits(v).ok_or(Decode::Invalid)
    }
}
