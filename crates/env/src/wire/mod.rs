//! envcall·wire — 环境调用载荷的字段 ↔ usize 契约（pack 与校验式 unpack）。
//!
//! 本文件 = **契约核心**：[`Wire`] trait、[`Decode`] 失败域、基元与权限位的 impl。
//! 字段**词汇表**按语义分居三个子模块（**声明次序即下表次序**，与下面的 `pub use` 同名）：
//!   - [`frompair`] —— 内核回写的 `(a0, a1)`（宽那一格 `a0..a2`）→ 域 Ret 载荷蒸馏；
//!   - [`handle`] —— 语义句柄（[`PieToken`] / [`TaskId`] / [`TeamId`] / [`VirtAddr`]）＋ 记号 [`Mark`]；
//!   - [`name`] —— 定长名字（[`NAME_LEN`] / [`Name`] / [`NameError`]）。
//!
//! **照实记（原先九个子模块，五个搬走、一个并掉）**：`args` / `key` / `manifest` / `pair` /
//! `supply` 从前也住这里，理由只是"都是两边要读的字节布局"。但**过线的东西**与**装机的账**
//! 不是同一件事（`pair.rs` 自己的头注就写着"这不是 envcall 载荷"），故它们随 `crates/plan`
//! 分了出去。**`access` 那一份并进了 [`permission`](crate::permission)**——`Access` / `Policy`
//! 是本文件头两段讲的那两个族（读写 / 传递）的视图类型，同一个故事没有理由分两处讲。
//!
//! **re-export 的口径**：可命名的类型一律在下面 re-export，故 `env::wire::Name` 与
//! `env::wire::name::Name` 两条路都在。
//!
//! 这是方案 3（typed payload）的**唯一类型擦除点**：每个字段类型都实现 [`Wire`]，
//! 由 [`derive(Envcall)`](mold) 生成的 codec 自动接线，用户侧与内核侧不再手写
//! `as usize` / `from_bits_truncate`。非法位校验收敛在此：`Permission` 的 unpack
//! 是 `from_bits(...).ok_or(...)`，而非静默截断。
//!
//! 通用性：本 trait 只依赖 `usize`，不绑 U-mode 语义——sbi 等 S-mode 调用封装
//! 未来可直接复用同一 codec（derive 不写死 envcall 路径）。
//!
//! **有三个 impl 的"类型在外"**：[`Permission`](crate::Permission) 在
//! [`permission`](crate::permission)、[`HoleDir`](crate::HoleDir) 与
//! [`ProgramKind`](crate::ProgramKind) 在 [`fid`](crate::fid)。这是**刻意**的：本仓的
//! 口径是"非法位校验**只有一处**"（上面那一句），故三个 impl 并排住这里，而不是各回各家。

pub mod frompair;
pub mod handle;
pub mod name;

pub use frompair::{FromPair, FromTriple};
pub use handle::{Mark, PieToken, TaskId, TeamId, VirtAddr};
pub use name::{NAME_LEN, Name, NameError};

/// 字段 ↔ usize 的契约。
pub mod field;

pub use field::{Field, fetch_bytes, fetch_tail, store_bytes, store_tail};

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
    /// 非法位（如 `Permission` 含未定义位，或 bool 非 0/1）。
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

impl Wire for crate::wait::Wait {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.to_wire();
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        // 满射（每个 `usize` 都有一格）⇒ 解码这一步没有非法位可拒。
        Ok(crate::wait::Wait::from_wire(v))
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
///
/// **超宽值同样要拒**：`a2`/`a3` 是整寄存器（`usize`），故「先 `as u32` 再校验」等于
/// 把 32 位以上静默丢掉后再判合法——`0x1_0000_0002` 会被解成 `STORE` 而不是 `Invalid`，
/// 那正是本条要根除的那类静默截断，只是搬到了高位。故先判宽度、再判位。
impl Wire for crate::permission::Permission {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.bits() as usize;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        let bits = u32::try_from(v).map_err(|_| Decode::Invalid)?;
        crate::permission::Permission::from_bits(bits).ok_or(Decode::Invalid)
    }
}
