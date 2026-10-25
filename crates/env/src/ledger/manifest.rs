//! initrd 的部署选择与程序清单。内核只读取 capsule 前言，用户态解释清单。
//!
//! ```text
//! [0..4]   entry_off u32
//! [4..8]   entry_len u32
//! [8..12]  magic "SQD1"
//! [12..16] count u32（1..=MAX_PROGRAMS）
//! [16..20] entry_at u32
//! [20..24] deployment_len u32
//! [24..]   deployment UTF-8
//! 每条：   [u32 kind][u32 name_len][name][u32 len][bytes]       （LE）
//! ```

use alloc::vec::Vec;

use crate::wire::program_kind::ProgramKind;

mod entries;
mod entry;
mod malformed;

pub use entries::Entries;
pub use entry::Entry;
pub use malformed::Malformed;

/// 清单条数上限——**读侧与写侧共用的一个上界**，不是"这一台的清单有几条"。
///
/// 故它退回成一个**明文上界**，而"加一行会不会撞上它"由**上层一条编译期断言**钉住：
/// `crates/composition/src/units/mod.rs` 里 `const _: () = assert!(PROGRAMS.len() <= MAX_PROGRAMS);`
/// ——超过就当场编不过。旧树那种"抬过一次、下一次没人回头看"的读法（它连着撞过三次：
/// 20 → 24 → 28 → 32）在这条断言下**不可能再静默发生**：要么编得过（上界还够），要么红。
///
/// 它偏大是故意的：本格是**防荒唐输入**的界（`count` 读到天文数字即判废），不是紧界
/// ——紧的界由写侧 `items.len()` 与读侧那段字节自己收。
pub const MAX_PROGRAMS: usize = 96;
/// 部署名和镜像名的字节上限。
const MAX_NAME: usize = 32;

/// **前言：给内核的两个数**——引导镜像在区内的偏移与长度。
///
/// 内核仍然**不认识清单格式**：它读的只是这 8 字节，清单本身还是域侧解释。
pub const PREAMBLE: usize = 8;

const MAGIC: &[u8; 4] = b"SQD1";
const HEAD: usize = PREAMBLE + 16;

/// 打包部署选择与镜像；`entry_at` 同时选择前言和清单中的引导项。
///
/// 返回值包含内核使用的 [`PREAMBLE`]；非法输入返回 `None`。
pub fn pack(
    items: &[(ProgramKind, &str, &[u8])],
    entry_at: usize,
    deployment: &str,
) -> Option<Vec<u8>> {
    if items.is_empty() || items.len() > MAX_PROGRAMS || entry_at >= items.len()
        || deployment.is_empty() || deployment.len() > MAX_NAME
    {
        return None;
    }
    let mut blob = Vec::new();
    blob.extend_from_slice(&0u32.to_le_bytes()); // entry_off —— 占位，下面回填
    blob.extend_from_slice(&0u32.to_le_bytes()); // entry_len
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&(items.len() as u32).to_le_bytes());
    blob.extend_from_slice(&(entry_at as u32).to_le_bytes());
    blob.extend_from_slice(&(deployment.len() as u32).to_le_bytes());
    blob.extend_from_slice(deployment.as_bytes());
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
    let entry = spans.get(entry_at)?;
    let (off, len) = (u32::try_from(entry.start).ok()?, u32::try_from(entry.len()).ok()?);
    blob[0..4].copy_from_slice(&off.to_le_bytes());
    blob[4..8].copy_from_slice(&len.to_le_bytes());
    Some(blob)
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
