//! rtc::call — **帧形与记号**：两句话、两种答形（纯函数，零依赖）。
//!
//! ```text
//!   问（客人 → 驱动）   [ASK]               →  [时刻 8B]        「现在几点」
//!                       [ARM][at 8B]        →  [答码 1B]        「在 at 叫我」
//!                                            … 到点那一声 [时刻 8B]
//!   回信孔（客人 ↔ 驱动）  记号 `rtc-back` —— 客人**每趟**铸一枚、借给驱动
//! ```
//!
//! **一问一答、一孔一趟**：报文里没有"往哪回"这一格——回答与那一声都走**这一趟自带的那枚
//! 孔**（号只在持有它的那张表里念得动，见 `protocol::session` 事实 8）。故帧里没有地址、
//! 没有身份，只有一个动作码。
//!
//! **答形由问形定、帧长可判**：1 字节是答码、8 字节是一个时刻。同一条 `ARM` 路上先到 1 字节
//! （收下了没有），后到 8 字节（到点那一声）。
//!
//! 本文件住**驱动自己那一片目录**，不在 `crates/protocol`：服务面 = 各驱动自己的具体协议
//! （那一条裁定见 `protocol::driver`），而它由驱动与客人**同一份源码**各 `use` 一次。
//! 可共用的只有形状——"失败域 ↔ 线上那一格"那张表用的是 `protocol::fail_codes!`。

use super::core::Fail;
use env::Mark;

/// 问那一句的动作码：「现在几点」。
pub const ASK: u8 = 1;

/// 问那一句的动作码：「在 `at` 叫我」。
pub const ARM: u8 = 2;

/// 一个时刻的字节数（u64 LE 纳秒）。
pub const TIME_LEN: usize = 8;

/// 答码的字节数。
pub const CODE_LEN: usize = 1;

/// `ASK` 帧的长度。
pub const ASK_LEN: usize = 1;

/// `ARM` 帧的长度（动作码 + 一个时刻）。
pub const ARM_LEN: usize = 1 + TIME_LEN;

/// 回信孔的记号：客人每趟铸一枚、借给驱动（回答与那一声都从它回来）。
pub const BACK: Mark = Mark::of("rtc-back");

/// 答话那一格：收下了。
pub const OK: u8 = 0;
/// 那一格有人了。
pub const TAKEN: u8 = 1;
/// 那个时刻已经过去了。
pub const PAST: u8 = 2;
/// 这一问读不懂 / 那一趟没走到。
///
/// **照实记**：本面只有两个"驱动说的话"（`TAKEN` / `PAST`），第三个失败格 `Fail::Denied`
/// 是**客侧自己判的**（推不进去、等到期、答话读不懂），它没有第二件要说的事，故与这一格
/// 合流——`system::board::call` 那张表里 `BAD` 在表外、`Denied` 另有 `DENIED` 一格，
/// 两处的差别就是"持有者那一侧会不会说出'我没接住'这句话"。
pub const BAD: u8 = 3;

protocol::fail_codes! {
    /// 失败域 → 答话那一格（**一处编**：客人那一侧与驱动那一侧看同一张表）。
    ///
    /// `None`（没失败）⇒ `OK`；反向（[`code_to_fail`]）只在双射时生成——本表是双射
    /// （三个失败三个码），故读的人不必另抄一份对照。
    bijective Fail; OK;
    Fail::Taken => TAKEN,
    Fail::Past => PAST,
    Fail::Denied => BAD,
}

/// 解出来的那一问。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ask {
    /// 「现在几点」。
    Now,
    /// 「在 `at` 叫我」（绝对时刻，纳秒）。
    Arm(u64),
}

/// 编一帧「现在几点」。
pub fn pack_ask() -> [u8; ASK_LEN] {
    [ASK]
}

/// 编一帧「在 at 叫我」。
pub fn pack_arm(at: u64) -> [u8; ARM_LEN] {
    let mut out = [0u8; ARM_LEN];
    out[0] = ARM;
    out[1..].copy_from_slice(&at.to_le_bytes());
    out
}

/// 拆一帧问：**不是那个形状就答 `None`**（别人往这扇门推别的东西时，不猜、不动账）。
pub fn unpack_ask(frame: &[u8]) -> Option<Ask> {
    match frame {
        [ASK] => Some(Ask::Now),
        [ARM, rest @ ..] => {
            let raw: [u8; TIME_LEN] = rest.try_into().ok()?;
            Some(Ask::Arm(u64::from_le_bytes(raw)))
        }
        _ => None,
    }
}

/// 编一个时刻。
pub fn pack_time(t: u64) -> [u8; TIME_LEN] {
    t.to_le_bytes()
}

/// 拆一个时刻：长度不对就答 `None`（答形的判据就是帧长）。
pub fn unpack_time(frame: &[u8]) -> Option<u64> {
    let raw: [u8; TIME_LEN] = frame.try_into().ok()?;
    Some(u64::from_le_bytes(raw))
}
