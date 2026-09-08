// initrd — 引导期程序清单（**临时机制**）。
//
// 位置与 `machine` 同级：同属「平台 / 引导供给」层，不进 `work::unit`（内核结构
// 核心）。退出路径：程序投递一旦有正式通道（运行期装载原语 / 设备发现），本模块
// 连同 `build.rs` 的打包端一起删除。
//
// 格式（LE，无对齐要求）：
//   [0..4]  count   u32   1..=MAX_PROGRAMS
//   每条：
//     [0..4]  name_len  u32   1..=MAX_NAME
//     [..]    name      ASCII，无 NUL
//     [0..4]  len       u32   >= 1
//     [..]    bytes     ELF 原样字节
//
// 无 magic：旧格式（裸 ELF）前 4 字节 0x464c_457f 远超 MAX_PROGRAMS，会被
// `TooMany` 当场拒掉——格式迁移期不需要额外标记。

use alloc::vec::Vec;

/// 清单条目上限（引导期程序数）。
pub(crate) const MAX_PROGRAMS: usize = 8;
/// 清单名字节上限。
pub(crate) const MAX_NAME: usize = 32;

/// 一条清单项（借用 blob）。
pub(crate) struct Program<'a> {
    pub name: &'a str,
    pub elf: &'a [u8],
}

/// 清单解析失败域（引导级致命——调用方 `expect`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitrdError {
    /// count == 0
    Empty,
    /// count > [`MAX_PROGRAMS`]
    TooMany { count: usize },
    /// 头 / 条目越界
    Truncated,
    /// name_len 越界、非 UTF-8 或含 NUL
    BadName,
    /// len == 0
    BadLen,
}

fn u32le(blob: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = blob.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// 解析程序清单；返回顺序即 blob 顺序（调用方按名取，不依赖顺序）。
pub(crate) fn programs(blob: &[u8]) -> Result<Vec<Program<'_>>, InitrdError> {
    let count = u32le(blob, 0).ok_or(InitrdError::Truncated)? as usize;
    if count == 0 {
        return Err(InitrdError::Empty);
    }
    if count > MAX_PROGRAMS {
        return Err(InitrdError::TooMany { count });
    }
    let mut out = Vec::with_capacity(count);
    let mut at = 4;
    for _ in 0..count {
        let name_len = u32le(blob, at).ok_or(InitrdError::Truncated)? as usize;
        at += 4;
        if name_len == 0 || name_len > MAX_NAME {
            return Err(InitrdError::BadName);
        }
        let raw = blob.get(at..at + name_len).ok_or(InitrdError::Truncated)?;
        let name = core::str::from_utf8(raw).map_err(|_| InitrdError::BadName)?;
        at += name_len;
        let len = u32le(blob, at).ok_or(InitrdError::Truncated)? as usize;
        at += 4;
        if len == 0 {
            return Err(InitrdError::BadLen);
        }
        let elf = blob.get(at..at + len).ok_or(InitrdError::Truncated)?;
        at += len;
        out.push(Program { name, elf });
    }
    Ok(out)
}

/// 按名取程序；未知名 → 打印清单后 panic（构建 / 引导不匹配当场暴露，不静默跳过）。
pub(crate) fn take<'a>(programs: &[Program<'a>], name: &str) -> &'a [u8] {
    match programs.iter().find(|p| p.name == name) {
        Some(p) => p.elf,
        None => {
            for p in programs {
                crate::putln!("[initrd] available: {}", p.name);
            }
            panic!("initrd: program `{name}` not found");
        }
    }
}
