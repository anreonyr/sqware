//! firmware::call — **线上形状**：单子上的一条（`Want`）、单子与回单的读写、上限与状态码——一个字节都不在别处编
//!
//! 正文见 [`super`]；记号、帧与上限见 [`crate::firmware::call`]。

use env::{NAME_LEN, Name, PAIR_LEN, TaskId};
use runtime::core::port::{Access, Policy};

use super::core::Fail;

/// 引导域↔编排域那条泊位的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const BOOT: &str = "boot";

/// 单子上"哪一类东西"那一格（**判别号即线格式**：`repr(u8)`）。
///
/// 三格对应内核那三种门闩句柄（`PolePie` / `NolePie` / `HolePie`）——固件据此挑对那一层。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 一段内存（设备寄存器页 / 自描述区 / 载荷区）。
    Pole,
    /// 空载荷的信号（中断门铃）。
    Nole,
    /// 一扇门（孔）：提示之路那一条由持树者铸、要经固件转手的孔。
    Hole,
}

/// 单子的操作码。今天只有"供"这一枚——留着这一格，是为加动作时不必改帧的布局。
pub const OP_SUPPLY: u8 = 1;

/// 一条单子最多要五样（今天的单子四样）。
pub const WANT_MAX: usize = 5;

/// 单子 / 回单的定长缓冲。
///
/// **不是线格式的上限**：孔不预设上限（见 `env::fid::PieCall::UnsealHole`），这两个数
/// 是本侧选"一帧一单、不流式"的结果。
pub const WANT_LEN: usize = core::mem::size_of::<Want>();
const _: () = assert!(WANT_LEN == 44);
/// 帧头：`[op][条数]` + 那一格"给谁"（8 字节 LE，与 `operator::tell` 同一口径）。
const HEAD_LEN: usize = 2 + 8;
pub const SLIP_CAP: usize = HEAD_LEN + WANT_LEN * WANT_MAX;
pub const REPLY_CAP: usize = 2 + PAIR_LEN * WANT_MAX;

/// 回单的状态码（与 operator / board 的码表同族）。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const DENIED: u8 = 2;
pub const FULL: u8 = 3;
pub const BAD: u8 = 4;

/// 单子上的一条：**要哪一枚**（boot 账里的名字）、什么种类、多少权、什么形态。
///
/// **收方那张需求单就是它的常量形态**（[`Want::of`] ＋ [`name_block`]，今天唯一一张在
/// `programs/src/supervisor/plic/needs.rs`）：收方开单、装配者原样递出。故这一条**不带
/// `slot`**——位置即格，收方按名字归位（`protocol::system::grant::unpack` 那个闭包）。
///
/// `repr(C)` + 定长字段 ⇒ 尺寸即线格式（编译期断言锁死），与 `Pair` 同一条纪律。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Want {
    name: [u8; NAME_LEN],
    access: u32,
    policy: u32,
    kind: u8,
    pad: [u8; 3],
}

/// 种类那一格的判别号（`Kind` 是 `repr(u8)`，故这就是线格式）。
const KIND_POLE: u8 = Kind::Pole as u8;
const KIND_NOLE: u8 = Kind::Nole as u8;
const KIND_HOLE: u8 = Kind::Hole as u8;

impl Want {
    /// 空的一条：填数组用（`name` 全零 = 非法名字，读侧一律答 `None`）。
    pub const NONE: Want = Want {
        name: [0u8; NAME_LEN],
        access: 0,
        policy: 0,
        kind: KIND_POLE,
        pad: [0u8; 3],
    };

    /// 按名要一样**不在设备需求单里**的东西（如提示之路、载荷区）。名字装不下 ⇒ `None`。
    pub fn new(name: &str, kind: Kind, access: Access, policy: Policy) -> Option<Want> {
        Some(Want {
            name: *Name::new(name).ok()?.bytes(),
            access: access.bits().bits(),
            policy: policy.bits().bits(),
            kind: kind as u8,
            pad: [0u8; 3],
        })
    }

    /// 常量构造：`name` 给的是**定长名字块**（[`Name::bytes`] 那一口径，见 [`name_block`]）。
    ///
    /// 运行期那条路是 [`Want::new`]；这一条是给**收方那张需求单**用的——它是一张 `const` 表
    /// （今天的唯一一张在 `programs/src/supervisor/plic/needs.rs`）。
    pub const fn of(name: [u8; NAME_LEN], kind: Kind, access: Access, policy: Policy) -> Want {
        Want {
            name,
            access: access.bits().bits(),
            policy: policy.bits().bits(),
            kind: kind as u8,
            pad: [0u8; 3],
        }
    }

    /// 名字（按 ABI 的定长字段解：`Name::bytes`/`from_bytes` 这一对，与 [`Pair::name`]
    /// 同一条——**不是**帧的变长那一条 `from_slice`，它会拒掉填充的 NUL）。
    pub fn name(&self) -> Option<Name> {
        Name::from_bytes(self.name).ok()
    }

    pub fn kind(&self) -> Option<Kind> {
        match self.kind {
            KIND_POLE => Some(Kind::Pole),
            KIND_NOLE => Some(Kind::Nole),
            KIND_HOLE => Some(Kind::Hole),
            _ => None,
        }
    }

    pub fn access(&self) -> Option<Access> {
        Access::from_bits(self.access)
    }

    /// 形态**原样**读出；"剔掉 `VEST`"是发货那一侧的事（见 [`supply`]）。
    pub fn policy(&self) -> Option<Policy> {
        Policy::from_bits(self.policy)
    }
}

/// 编译期把字面量补零成定长名字块——与 [`Name::new`] 运行期做的是同一件事。
///
/// **不做** `Name::new` 那套校验（非空 / UTF-8 / ≤ `NAME_LEN` - 1）：它是给 `const` 表用的，
/// 字面量写错会在 [`Name::from_bytes`] 读出时当场暴露（表是编译期常量，读的人就在旁边）。
pub const fn name_block(s: &str) -> [u8; NAME_LEN] {
    let src = s.as_bytes();
    let mut out = [0u8; NAME_LEN];
    let mut i = 0;
    while i < src.len() && i < NAME_LEN - 1 {
        out[i] = src[i];
        i += 1;
    }
    out
}

/// 起一张单子 → 帧。条数越界或缓冲不够 ⇒ `None`（调用方按本地失败处理）。
pub fn slip<'a>(buf: &'a mut [u8], who: TaskId, wants: &[Want]) -> Option<&'a [u8]> {
    let n = wants.len();
    if n > WANT_MAX {
        return None;
    }
    let len = HEAD_LEN + n * WANT_LEN;
    let out = buf.get_mut(..len)?;
    out[0] = OP_SUPPLY;
    out[1] = n as u8;
    out[2..HEAD_LEN].copy_from_slice(&(who.get() as u64).to_le_bytes());
    for (i, w) in wants.iter().enumerate() {
        let at = HEAD_LEN + i * WANT_LEN;
        out[at..at + WANT_LEN].copy_from_slice(want_bytes(w));
    }
    Some(out)
}

/// 读一张单子。`None` = 帧读不懂（op 不对 / 条数越界 / 缓冲短）。
pub fn slip_of(bytes: &[u8]) -> Option<Slip<'_>> {
    if bytes.len() < HEAD_LEN || bytes[0] != OP_SUPPLY {
        return None;
    }
    let n = bytes[1] as usize;
    if n > WANT_MAX || bytes.len() < HEAD_LEN + n * WANT_LEN {
        return None;
    }
    Some(Slip { bytes })
}

/// 读出来的一张单子（借字节）。逐条 `read_unaligned`——缓冲只保证 1 字节对齐
/// （与 `Pair` 同一条理由：线格式的步长是 44，而块只保证页对齐）。
pub struct Slip<'a> {
    bytes: &'a [u8],
}

impl<'a> Slip<'a> {
    /// 这条单子是**给谁**的（那一格过线的号）。
    pub fn who(&self) -> TaskId {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&self.bytes[2..HEAD_LEN]);
        TaskId::new(u64::from_le_bytes(raw) as usize)
    }

    pub fn len(&self) -> usize {
        self.bytes[1] as usize
    }

    pub fn want(&self, i: usize) -> Option<Want> {
        if i >= self.len() {
            return None;
        }
        let at = HEAD_LEN + i * WANT_LEN;
        // SAFETY: 区间由 `len()` 与 `slip_of` 的长度校验保证在缓冲内；缓冲只保证
        // 1 字节对齐，故 `read_unaligned`。
        Some(unsafe { core::ptr::read_unaligned(self.bytes.as_ptr().add(at).cast::<Want>()) })
    }
}

/// 编一张回单：一格状态 + 若干条 [`Pair`] 记录。
///
/// `records` 必须是整条记录（`PAIR_LEN` 步长），条数越界或缓冲不够 ⇒ `None`。
pub fn reply<'a>(buf: &'a mut [u8], code: u8, records: &[u8]) -> Option<&'a [u8]> {
    if !records.len().is_multiple_of(PAIR_LEN) {
        return None;
    }
    let n = records.len() / PAIR_LEN;
    if n > WANT_MAX {
        return None;
    }
    let out = buf.get_mut(..2 + records.len())?;
    out[0] = code;
    out[1] = n as u8;
    out[2..].copy_from_slice(records);
    Some(out)
}

/// 读回单的状态。`None` = 帧读不懂。
pub fn code_of(bytes: &[u8]) -> Option<u8> {
    let head = bytes.get(..2)?;
    let rest = bytes.get(2..)?;
    if rest.len() != head[1] as usize * PAIR_LEN {
        return None;
    }
    Some(head[0])
}

/// 读回单里的记录——**就是原样交给客人的那一段**。
pub fn records_of(bytes: &[u8]) -> Option<&[u8]> {
    let n = *bytes.get(1)? as usize;
    if n > WANT_MAX {
        return None;
    }
    let rest = bytes.get(2..)?;
    if rest.len() != n * PAIR_LEN {
        return None;
    }
    Some(rest)
}

/// 线上状态码 → 本地失败域。`OK` 不是失败，故返 `None`。
pub const fn fail_of(code: u8) -> Option<Fail> {
    match code {
        UNKNOWN => Some(Fail::Unknown),
        DENIED => Some(Fail::Denied),
        FULL => Some(Fail::Full),
        BAD => Some(Fail::Bad),
        _ => None,
    }
}

/// 本地失败域 → 线上状态码。
pub const fn code_of_fail(fail: Fail) -> u8 {
    match fail {
        Fail::Local | Fail::Bad => BAD,
        Fail::Unknown => UNKNOWN,
        Fail::Denied => DENIED,
        Fail::Full => FULL,
    }
}

/// 单子上那一条的字节——与 [`pair_bytes`] 同一条理由（`repr(C)`、尺寸编译期锁死）。
fn want_bytes(want: &Want) -> &[u8; WANT_LEN] {
    // SAFETY: `Want` 是 `repr(C)`、尺寸由编译期断言等于 `WANT_LEN`，只读解释为字节安全。
    unsafe { &*(want as *const Want).cast::<[u8; WANT_LEN]>() }
}
