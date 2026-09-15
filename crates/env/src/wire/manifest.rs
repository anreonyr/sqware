//! initrd 清单——boot 交给 root 的**程序账**（一条 = 一个可装载程序）。
//!
//! 打包的一侧是内核的 `build.rs`（宿主程序），读的一侧是 root 域，故格式在此定义一次
//! （与 [`pair`](super::pair) 同一条理由：跨域的字节布局不留第二份账）。
//!
//! ```text
//! [0..4]  count u32（1..=MAX_PROGRAMS）
//! 每条：  [u32 kind][u32 name_len][name][u32 len][bytes]        （LE）
//! ```
//!
//! `kind` 是 [`ProgramKind`] 的码（0 = `User`、1 = `Supervisor`）——**特权级的唯一声明
//! 处是内核的打包表**（`kernel/build.rs::INITRD_BINS`），域只读取并原样转交 `Build`。
//!
//! 内核**不解释**这份清单：它只按打包期常量取出引导镜像（`kernel/src/platform/initrd.rs`）。
//! 写侧（[`pack`]）与读侧（[`Entries`]）共用同一批判据，故写侧不可能产出读侧拒收的清单。

use alloc::vec::Vec;
use core::ops::Range;

use crate::fid::ProgramKind;

/// 清单条数上限。
pub const MAX_PROGRAMS: usize = 16;
/// 一条记录里名字的字节上限（与 [`Name`](super::Name) 同值：名字要能原样进 `Team.name`）。
pub const MAX_NAME: usize = 32;

/// 清单头（`count`）的字节数。
const HEAD: usize = 4;

/// 清单里的一条（借用清单区）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry<'a> {
    /// 装成哪种空间（`Build` 的特权级参数）。
    pub kind: ProgramKind,
    /// 清单名（root 按它挑程序）。
    pub name: &'a str,
    /// 镜像字节。
    pub elf: &'a [u8],
}

/// 一条记录非法：越界 / `kind` 未知 / 名字空或超限 / 镜像为空 / 非 UTF-8 名字。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Malformed;

/// 逐条读清单（读侧：域）。[`new`](Entries::new) 只读头并校验条数；每条都做全量校验，
/// 非法即当场答 `Err(Malformed)`——**不 panic，也不猜**。
pub struct Entries<'a> {
    blob: &'a [u8],
    at: usize,
    left: usize,
}

impl<'a> Entries<'a> {
    /// 读头：返回游标；`None` = 清单头非法（空清单 / 超上限 / 装不下）。
    pub fn new(blob: &'a [u8]) -> Option<Self> {
        let left = u32le(blob, 0)? as usize;
        if left == 0 || left > MAX_PROGRAMS {
            return None;
        }
        Some(Self {
            blob,
            at: HEAD,
            left,
        })
    }
}

impl<'a> Iterator for Entries<'a> {
    type Item = Result<Entry<'a>, Malformed>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        Some(read_one(self.blob, &mut self.at))
    }
}

/// 打包（写侧：内核的 `build.rs`）：清单区字节 + **每条在区内的字节区间**（与 `items` 同序）。
///
/// 判据与读侧同一份（条数 / 名字长度 / 空镜像），非法 → `None`。
pub fn pack(items: &[(ProgramKind, &str, &[u8])]) -> Option<(Vec<u8>, Vec<Range<usize>>)> {
    if items.is_empty() || items.len() > MAX_PROGRAMS {
        return None;
    }
    let mut blob = Vec::new();
    blob.extend_from_slice(&(items.len() as u32).to_le_bytes());
    let mut spans = Vec::with_capacity(items.len());
    for (kind, name, elf) in items {
        if name.is_empty() || name.len() > MAX_NAME || elf.is_empty() {
            return None;
        }
        blob.extend_from_slice(&code(*kind).to_le_bytes());
        blob.extend_from_slice(&(name.len() as u32).to_le_bytes());
        blob.extend_from_slice(name.as_bytes());
        blob.extend_from_slice(&(elf.len() as u32).to_le_bytes());
        let start = blob.len();
        blob.extend_from_slice(elf);
        spans.push(start..blob.len());
    }
    Some((blob, spans))
}

/// 读一条（游标前进）；任何非法 → `Err(Malformed)`。
fn read_one<'a>(blob: &'a [u8], at: &mut usize) -> Result<Entry<'a>, Malformed> {
    let kind = match u32le(blob, *at) {
        Some(c) => match kind_of(c) {
            Some(k) => k,
            None => return Err(Malformed),
        },
        None => return Err(Malformed),
    };
    *at += 4;
    let name_len = match u32le(blob, *at) {
        Some(v) => v as usize,
        None => return Err(Malformed),
    };
    *at += 4;
    if name_len == 0 || name_len > MAX_NAME {
        return Err(Malformed);
    }
    let name = match blob
        .get(*at..*at + name_len)
        .and_then(|b| core::str::from_utf8(b).ok())
    {
        Some(s) => s,
        None => return Err(Malformed),
    };
    *at += name_len;
    let len = match u32le(blob, *at) {
        Some(v) => v as usize,
        None => return Err(Malformed),
    };
    *at += 4;
    if len == 0 {
        return Err(Malformed);
    }
    let elf = match blob.get(*at..*at + len) {
        Some(b) => b,
        None => return Err(Malformed),
    };
    *at += len;
    Ok(Entry { kind, name, elf })
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    let v: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(v))
}

/// `ProgramKind` → 清单里的码。**码只在这里写死一次**（打包表的类型即 `ProgramKind`）。
const fn code(kind: ProgramKind) -> u32 {
    match kind {
        ProgramKind::User => 0,
        ProgramKind::Supervisor => 1,
    }
}

fn kind_of(code: u32) -> Option<ProgramKind> {
    match code {
        0 => Some(ProgramKind::User),
        1 => Some(ProgramKind::Supervisor),
        _ => None,
    }
}
