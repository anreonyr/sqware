//! 清单解析（root 私有）：格式由 `kernel/build.rs` 打包，**内核不解释它**。
//!
//! ```text
//! [0..4] count u32
//! 每条：[u32 kind][u32 name_len][name][u32 len][bytes]        （LE）
//! ```
//!
//! `kind` 来自内核的打包表（`build.rs::INITRD_BINS`），不是程序自述——root 只
//! 读取并原样转交 `Build`（见 `docs/supervisor.md` §13）。

use alloc::vec::Vec;

use env::ProgramKind;

/// 一条清单项（借用清单区）。
pub struct Entry<'a> {
    pub name: &'a str,
    pub kind: ProgramKind,
    pub elf: &'a [u8],
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    let v: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(v))
}

/// 解析清单；任何越界/非法值 → None（root 据此拒绝启动）。
pub fn parse(blob: &[u8]) -> Option<Vec<Entry<'_>>> {
    let count = u32le(blob, 0)? as usize;
    if count == 0 || count > 16 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut at = 4usize;
    for _ in 0..count {
        let kind = match u32le(blob, at)? {
            0 => ProgramKind::User,
            1 => ProgramKind::Supervisor,
            _ => return None,
        };
        at += 4;
        let name_len = u32le(blob, at)? as usize;
        at += 4;
        if name_len == 0 || name_len > 32 {
            return None;
        }
        let name = core::str::from_utf8(blob.get(at..at + name_len)?).ok()?;
        at += name_len;
        let len = u32le(blob, at)? as usize;
        at += 4;
        if len == 0 {
            return None;
        }
        let elf = blob.get(at..at + len)?;
        at += len;
        out.push(Entry { name, kind, elf });
    }
    Some(out)
}
