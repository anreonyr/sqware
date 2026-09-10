//! 定长名字——域名字与目录协议共用的一个上限。
//!
//! [`NAME_LEN`] 是**单一真相**：内容 ≤ 31 字节 + 终止 NUL = 32；`dispatch` 的
//! 目录协议与 `Team.name` 共用它。`Name` 把「非空、≤ 31 字节、不含 NUL」做成
//! 构造期义务，非法输入由 [`NameError`] 承载——不 panic、不截断。

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
