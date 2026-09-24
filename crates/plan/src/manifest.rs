//! initrd 清单——boot 交给 root 的**程序账**（一条 = 一个可装载程序）。
//!
//! 打包的一侧是内核的 `build.rs`（宿主程序），读的一侧是域（引导域读它挑引导镜像，编排域
//! 读它挑各服务的镜像——同一批字节，见 `platform/devices.rs::supply_initrd`），故格式在此定义一次
//! （与 [`pair`](crate::pair) 同一条理由：跨域的字节布局不留第二份账）。
//!
//! ```text
//! [0..4]   root_off u32        ← **给内核的两个数**（见 [`PREAMBLE`]）
//! [4..8]   root_len u32
//! [8..12]  count u32（1..=MAX_PROGRAMS）
//! 每条：   [u32 kind][u32 name_len][name][u32 len][bytes]       （LE）
//! ```
//!
//! `kind` 是 [`ProgramKind`] 的码（0 = `User`、1 = `Supervisor`）——**特权级的唯一声明
//! 处是装配单**（`plan::assembly::ALL` 里那一行的 `kind`；打包那一侧是 `crates/image`），
//! 域只读取并原样转交 `Build`。
//!
//! 内核**不解释**这份清单：它只读前 8 字节那两个数取引导镜像（`kernel/src/platform/initrd.rs`）。
//! 写侧（[`pack`]）与读侧（[`Entries`]）共用同一批判据，故写侧不可能产出读侧拒收的清单。

use alloc::vec::Vec;

use env::ProgramKind;

/// 清单条数上限。
///
/// 清单条数上限 ＝ **装配单的行数**（[`crate::assembly::ALL`]）。
///
/// **它不是人挑的数，是数出来的**：一张镜像的清单是装配单某一景的**子集**（每行自己的
/// `scenes` 说了它进哪几张），故"一景有几台 ≤ 表有几行"**由构造成立** ⇒ 取 `ALL.len()`
/// 永远够用，而 `pack`（写侧）与 [`Entries::new`]（读侧）本来读的就是同一格。
///
/// **照实记（这一格连着撞过三次，最后是"清单不再取并集"收的口）**：它原先写死。20——理由那句
/// 是"今天的 17 个程序"，而 17 是写那句话时的数，后来加到 20 个时没人回头看它，于是"留三格
/// 余量"从那一天起就是假的（20 正好卡满）；那一刀（`/device/rtc` 那面服务带来第二十一个程序）
/// 抬到 24；再一刀（结盟服务 ＋ 它的探针）抬到 28；又一边（规矩那一格的两台证客）撞满 ⇒
/// 抬到 32——**三次都是"+3"**，三次症状都一样（`pack` 返 `None`）。
///
/// 压力的成因是"清单 = 全部程序的并集"，而**用户裁定"测试和程序分开"**那一刀之后，清单变成
/// **按景取的子集**了。**本笔收口**：不再由人抬——往装配单加一行，这个数自己长。
pub const MAX_PROGRAMS: usize = crate::assembly::ALL.len();
/// 一条记录里名字的字节上限（清单里的**字节段**长度；`Name` 的内容上限是 31——
/// 清单名是给装配账看的，不必进一枚 `Name`，内核更不收它）。
///
/// **不是 `pub`**（照实记）：全仓只有本文件两处读它（写侧 `pack`、读侧 `read_one`）——
/// 打包那一侧（`crates/image`）调 `pack` 就够了，它不必知道这个界。`pub` 会是一扇没有读者的门。
const MAX_NAME: usize = 32;

/// **前言：给内核的两个数**——引导镜像在区内的偏移与长度。
///
/// **照实记（为什么有这 8 字节）**：内核原先按 `env!("ROOT_OFFSET")` / `ROOT_LEN` 两个**打包期
/// 常量**取引导镜像，于是**打包结果要回喂内核源码**——改一个客人也会让内核重编（实测：一次
/// 1.3 s），而且 `cargo build` 不可能只编内核。把这两个数写进区里之后，内核**开机读**它：
/// 编译期常量取消，那条反馈边断了（"initrd 与内核何干"那一问的答案就在这里）。
///
/// 内核仍然**不认识清单格式**：它读的只是这 8 字节，清单本身还是域侧解释。
pub const PREAMBLE: usize = 8;

/// 清单头（`count`）的字节数——在前言之后。
const HEAD: usize = PREAMBLE + 4;

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
        let left = u32le(blob, PREAMBLE)? as usize;
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

/// 打包（写侧：内核的 `build.rs`）：清单区字节（**含 [`PREAMBLE`] 那 8 字节**）。
///
/// `root_at` = **引导镜像**在 `items` 里的下标：前言那两格按它算（布局完了回填）。
/// 判据与读侧同一份（条数 / 名字长度 / 空镜像），非法 → `None`。
pub fn pack(items: &[(ProgramKind, &str, &[u8])], root_at: usize) -> Option<Vec<u8>> {
    if items.is_empty() || items.len() > MAX_PROGRAMS || root_at >= items.len() {
        return None;
    }
    let mut blob = Vec::new();
    blob.extend_from_slice(&0u32.to_le_bytes()); // root_off —— 占位，下面回填
    blob.extend_from_slice(&0u32.to_le_bytes()); // root_len
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
    let root = spans.get(root_at)?;
    let (off, len) = (root.start as u32, root.len() as u32);
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
