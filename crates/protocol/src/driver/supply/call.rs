//! supply::call — **线上形状**：单子上的一条（[`Want`]）与收方那张 `const` 表里的一格（[`Need`]）、
//! 单子与回单的编解、上限与状态码——一个字节都不在别处编
//!
//! 正文见 [`super`]；记号、帧与上限见 [`crate::driver::supply::call`]。

use env::{PAIR_LEN, TaskId};

use super::core::Fail;

/// 引导域↔编排域那条泊位的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const BOOT: &str = "boot";



// **词汇搬去 `env` 了**（装配单要宿主侧也读得到，见 `env::wire::supply` 头注）：本处只转发，
// **调用点一行没改**。
pub use env::wire::supply::{At, Kind, Need, Want, class_block};

/// 单子的操作码。今天只有"供"这一枚——留着这一格，是为加动作时不必改帧的布局。
pub const OP_SUPPLY: u8 = 1;

/// 一条单子最多要五样（今天的单子四样）。
pub const WANT_MAX: usize = 5;

/// 单子 / 回单的定长缓冲。
///
/// **不是线格式的上限**：孔不预设上限（见 `env::fid::PieCall::UnsealHole`），这两个数
/// 是本侧选"一帧一单、不流式"的结果。
pub const WANT_LEN: usize = core::mem::size_of::<Want>();
const _: () = assert!(WANT_LEN == 32);
/// **各项之和就是它**——留一格隐式的尾巴，`want_bytes` 就会把未初始化字节读上线（见 `Want` 的注）。
const _: () = assert!(core::mem::size_of::<Want>() == env::KEY_LEN + 4 + 4 + 1 + 7);
/// 帧头：`[op][条数]` + 那一格"给谁"（8 字节 LE，与 `operator::tell` 同一口径）。
const HEAD_LEN: usize = 2 + 8;
pub const ORDER_CAP: usize = HEAD_LEN + WANT_LEN * WANT_MAX;
pub const REPLY_CAP: usize = 2 + PAIR_LEN * WANT_MAX;

/// 回单的状态码（与 operator / system::board 的码表同族）。
pub const OK: u8 = 0;
pub const UNKNOWN: u8 = 1;
pub const DENIED: u8 = 2;
pub const FULL: u8 = 3;
pub const BAD: u8 = 4;



/// 种类那一格的判别号（`Kind` 是 `repr(u8)`，故这就是线格式）。











/// 起一张单子 → 帧。条数越界或缓冲不够 ⇒ `None`（调用方按本地失败处理）。
pub fn pack_order<'a>(buf: &'a mut [u8], who: TaskId, wants: &[Want]) -> Option<&'a [u8]> {
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
pub fn unpack_order(bytes: &[u8]) -> Option<Order<'_>> {
    if bytes.len() < HEAD_LEN || bytes[0] != OP_SUPPLY {
        return None;
    }
    let n = bytes[1] as usize;
    if n > WANT_MAX || bytes.len() < HEAD_LEN + n * WANT_LEN {
        return None;
    }
    Some(Order { bytes })
}

/// 读出来的一张单子（借字节）。逐条 `read_unaligned`——缓冲只保证 1 字节对齐
/// （与 `Pair` 同一条理由：线格式的步长是 32，而块只保证页对齐）。
pub struct Order<'a> {
    bytes: &'a [u8],
}

impl<'a> Order<'a> {
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
        // SAFETY: 区间由 `len()` 与 `unpack_order` 的长度校验保证在缓冲内；缓冲只保证
        // 1 字节对齐，故 `read_unaligned`。
        Some(unsafe { core::ptr::read_unaligned(self.bytes.as_ptr().add(at).cast::<Want>()) })
    }
}

/// 编一张回单：一格状态 + 若干条 [`Pair`] 记录。
///
/// `records` 必须是整条记录（`PAIR_LEN` 步长），条数越界或缓冲不够 ⇒ `None`。
pub fn pack_reply<'a>(buf: &'a mut [u8], code: u8, records: &[u8]) -> Option<&'a [u8]> {
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

/// 读一张回单：形状不对 ⇒ `None`（不猜、不崩）。
pub fn unpack_reply(bytes: &[u8]) -> Option<Reply<'_>> {
    let head = bytes.get(..2)?;
    let n = head[1] as usize;
    if n > WANT_MAX || bytes.len() != 2 + n * PAIR_LEN {
        return None;
    }
    Some(Reply { bytes })
}

/// 读出来的一张回单（借字节）。两格：**答话那一格**与**记录那一段**。
///
/// 与 [`Order`] 同一条形状：编解是**一对自由函数**（[`pack_reply`] / [`unpack_reply`]），
/// 取格是**视图上的名词方法**——`_of` 后缀只留给"从裸字节里取一格"的查询。
pub struct Reply<'a> {
    bytes: &'a [u8],
}

impl<'a> Reply<'a> {
    /// 答话那一格（[`OK`] / [`UNKNOWN`] / [`DENIED`] / [`FULL`] / [`BAD`]）。
    pub fn code(&self) -> u8 {
        self.bytes[0]
    }

    /// 记录那一段——**就是原样交给客人的那一段**（整条记录，`PAIR_LEN` 步长）。
    pub fn records(&self) -> &'a [u8] {
        &self.bytes[2..]
    }
}

/// 线上状态码 → 本地失败域。`OK` 不是失败，故返 `None`。
///
/// **本表不是双射**：`Fail::Local` 与 `Fail::Bad` 归同一个 `BAD`，故这一手是**尽力而为**的
/// 反向——`BAD` 只答得回 `Fail::Bad`，`Local` 一去不回。**这一条是成文的**：码表宏不给
/// 非双射的表生成反向，这一手因此由人写在这里。
pub const fn code_to_fail(code: u8) -> Option<Fail> {
    match code {
        UNKNOWN => Some(Fail::Unknown),
        DENIED => Some(Fail::Denied),
        FULL => Some(Fail::Full),
        BAD => Some(Fail::Bad),
        _ => None,
    }
}

fail_codes! {
    /// 本地失败域 → 线上状态码。`None`（没失败）⇒ `OK`——与下面那个 [`code_to_fail`] 的
    /// `OK ⇒ None` 正好是同一格的两侧读法。
    ///
    /// **本表不是双射**（`Local` 与 `Bad` 同归 `BAD`），故宏**不给反向**：反向由人写在下面，
    /// 并注明它反不回来。
    lossy Fail; OK;
    Fail::Local | Fail::Bad => BAD,
    Fail::Unknown => UNKNOWN,
    Fail::Denied => DENIED,
    Fail::Full => FULL,
}

/// 单子上那一条的字节——与 [`pair_bytes`] 同一条理由（`repr(C)`、尺寸编译期锁死）。
fn want_bytes(want: &Want) -> &[u8; WANT_LEN] {
    // SAFETY: `Want` 是 `repr(C)`、尺寸由编译期断言等于 `WANT_LEN`，只读解释为字节安全。
    unsafe { &*(want as *const Want).cast::<[u8; WANT_LEN]>() }
}
