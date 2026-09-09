//! envcall·wire — 环境调用载荷的字段 ↔ usize 契约（package/pack 与校验式 unpack）。
//!
//! 这是方案 3（typed payload）的**唯一类型擦除点**：每个字段类型都实现 [`Wire`]，
//! 由 [`derive(Envcall)`](envmacros) 生成的 codec 自动接线，用户侧与内核侧不再手写
//! `as usize` / `from_bits_truncate`。非法位校验收敛在此：`Permission`/`PteFlags`
//! 的 unpack 是 `from_bits(...).ok_or(...)`，而非静默截断。
//!
//! 通用性：本 trait 只依赖 `usize`，不绑 U-mode 语义——sbi 等 S-mode 调用封装
//! 未来可直接复用同一 codec（derive 不写死 envcall 路径）。

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

// ── 语义句柄 ────────────────────────────────────────────────────────────

/// per-pie 全局唯一句柄（u64；0 = 无效哨兵）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct PieToken(pub u64);

impl PieToken {
    pub const fn new(v: u64) -> Self {
        Self(v)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<PieToken> for u64 {
    fn from(t: PieToken) -> Self {
        t.0
    }
}

impl Wire for PieToken {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0 as usize;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)? as u64;
        *i += 1;
        Ok(PieToken(v))
    }
}

/// 任务句柄（全局唯一 id；0 = 无上下文）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct TaskId(pub usize);

impl TaskId {
    pub const fn new(v: usize) -> Self {
        Self(v)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<TaskId> for usize {
    fn from(t: TaskId) -> Self {
        t.0
    }
}

impl Wire for TaskId {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(TaskId(v))
    }
}

/// 用户虚拟地址 / 窗口指针（envcall 侧仅作 ABI 语义标记；内核侧以 `VirtAddr::from_raw`
/// 转入具体地址语义）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct VirtAddr(pub usize);

impl VirtAddr {
    pub const fn new(v: usize) -> Self {
        Self(v)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<VirtAddr> for usize {
    fn from(v: VirtAddr) -> Self {
        v.0
    }
}

impl Wire for VirtAddr {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(VirtAddr(v))
    }
}

/// 团队句柄（域标识；0 = 无效哨兵）。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub struct TeamId(pub usize);

impl TeamId {
    pub const fn new(v: usize) -> Self {
        Self(v)
    }
    pub const fn get(self) -> usize {
        self.0
    }
}

impl From<TeamId> for usize {
    fn from(t: TeamId) -> Self {
        t.0
    }
}

impl Wire for TeamId {
    fn pack(&self, s: &mut [usize; 6], i: &mut usize) {
        s[*i] = self.0;
        *i += 1;
    }
    fn unpack(s: &[usize; 6], i: &mut usize) -> Result<Self, Decode> {
        let v = *s.get(*i).ok_or(Decode::Overflow)?;
        *i += 1;
        Ok(TeamId(v))
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

/// 由内核回写的 `(a0, a1)` 还原「域 Ret 载荷」的契约（R3 蒸馏）。
///
/// derive(Envcall) 生成的 `call()` 在正（非负）路径按 variant 调用
/// `<T as FromPair>::from_pair(v0, v1)` 组装 Ret 载荷；错误路径已在
/// `EnvError::from_raw` 分支，故此处只见成功值。
pub trait FromPair: Sized {
    fn from_pair(v0: usize, v1: usize) -> Self;
}

impl FromPair for () {
    fn from_pair(_v0: usize, _v1: usize) -> Self {}
}

impl FromPair for usize {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0
    }
}

impl FromPair for u64 {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0 as u64
    }
}

impl FromPair for (u64, u64) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (v0 as u64, v1 as u64)
    }
}

impl FromPair for (PieToken, PieToken) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (PieToken(v0 as u64), PieToken(v1 as u64))
    }
}

impl FromPair for (PieToken, crate::permission::Permission) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        (
            PieToken(v0 as u64),
            crate::permission::Permission::from_bits_truncate(v1 as u32),
        )
    }
}

/// Collect 返回值打包：v0 = token（u64），v1 低 32 位 = permission bits、v1 高 32 位 = vestor task id。
/// vestor = None 由内核编码为 `TaskId(0)`（哨兵与原 vestor=None 语义一致）。
impl FromPair for (PieToken, crate::permission::Permission, TaskId) {
    fn from_pair(v0: usize, v1: usize) -> Self {
        let permission = crate::permission::Permission::from_bits_truncate(v1 as u32);
        let vestor = TaskId((v1 >> 32) as usize);
        (PieToken(v0 as u64), permission, vestor)
    }
}

impl FromPair for bool {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0 != 0
    }
}

impl FromPair for u8 {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        v0 as u8
    }
}

impl FromPair for PieToken {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        PieToken(v0 as u64)
    }
}

impl FromPair for TaskId {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        TaskId(v0)
    }
}

impl FromPair for TeamId {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        TeamId(v0)
    }
}

impl FromPair for VirtAddr {
    fn from_pair(v0: usize, _v1: usize) -> Self {
        VirtAddr(v0)
    }
}

// ── 定长名字 ────────────────────────────────────────────────────────────

/// 名字字段字节数（含终止 NUL）。
///
/// 单一真相：目录协议（`dispatch`）与域名字（`Team.name`）共用同一上限——
/// 内容 ≤ 31 字节。
pub const NAME_LEN: usize = 32;

/// 名字校验失败域。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameError {
    /// 空名。
    Empty,
    /// 超出 [`NAME_LEN`] - 1 字节（要留终止 NUL）。
    TooLong,
    /// 含 NUL（会与填充歧义）。
    Nul,
}

/// 定长名字：32 字节、尾随 NUL 填充、内容非空且不含 NUL。
///
/// 类型义务：非法名不可表达——拿到 `Name` 即已校验，调用方不再查；比较按整块
/// 定长字节（填充由构造保证规范，故等值即语义等值）。`Hash`/`Ord` 供目录容器
/// 与排序枚举使用。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Name {
    bytes: [u8; NAME_LEN],
}

impl Name {
    /// 由字符串构造（校验失败即拒绝，不截断）。
    pub fn new(s: &str) -> Result<Name, NameError> {
        let b = s.as_bytes();
        if b.is_empty() {
            return Err(NameError::Empty);
        }
        if b.len() >= NAME_LEN {
            return Err(NameError::TooLong);
        }
        if b.contains(&0) {
            return Err(NameError::Nul);
        }
        let mut bytes = [0u8; NAME_LEN];
        bytes[..b.len()].copy_from_slice(b);
        Ok(Name { bytes })
    }

    /// 由线上字节还原（校验填充规范 + 内容合法）。`dispatch` 的 decode 用。
    pub(crate) fn from_bytes(bytes: [u8; NAME_LEN]) -> Result<Name, NameError> {
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        if len == 0 {
            return Err(NameError::Empty);
        }
        if bytes[len..].iter().any(|&b| b != 0) {
            return Err(NameError::Nul);
        }
        if core::str::from_utf8(&bytes[..len]).is_err() {
            return Err(NameError::Nul);
        }
        Ok(Name { bytes })
    }

    /// 定长字节视图（含填充）。
    pub fn bytes(&self) -> &[u8; NAME_LEN] {
        &self.bytes
    }

    /// 内容长度（终止 NUL 之前）。
    pub fn len(&self) -> usize {
        self.bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 名字文本（构造已保证 UTF-8）。
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len()]).unwrap_or("")
    }
}

impl core::fmt::Display for Name {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}
