//! rtc::call — **帧形与记号**：两句话、两种答形（纯函数，零依赖）。
//!
//! ```text
//!   问（客人 → 驱动）   [ASK][那一格 8B]              →  [时刻 8B]     「现在几点」
//!                       [ARM][那一格 8B][at 8B]       →  [答码 1B]     「在 at 叫我」
//!                                                     … 到点那一声 [时刻 8B]
//!   回信孔（客人 ↔ 驱动）  记号 `rtc-back` —— 客人**每趟**铸一枚、借给驱动，
//!                          并把"它**在驱动表里**是几号"写进帧
//! ```
//!
//! **"往哪回"那一格现在在帧里**（`[1..9]`）。**照实记（这是裁定甲′翻掉的旧口径）**：旧写法
//! 是"报文里没有'往哪回'这一格——回答走那一趟自带的那枚孔，收方按'谁给的 ＋ 记号'**扫全表**
//! 认回来"。那次拒绝**没有读数垫底**；量出来之后：收方那一扫是**每帧 ~6.5 ms**（表 16 枚
//! ⇒ 扫一遍 + 每枚一次 `reserve` = O(n²)），而"那一枚在你表里是几号"（`port::ship` 的
//! `to.seed()`）**本来就算出来了、只是被丢掉**。故客人把它写进帧，收方**一次 `reserve` 验
//! 一下**就用。
//!
//! **那一格不是凭证，是一次验**：收方不许拿它直接写信——必须 `reserve` 出 `(owner, 记号)`
//! 并核对 `owner == 发信人 && 记号 == rtc-back`，否则客人能让驱动往**别人的孔**里写。
//! （旧那一扫天然带这条核对；换法之后它是**显式**的一步，不是免费的了。）
//!
//! 帧里仍然没有地址、没有身份：那一格是**收方自己表里**的号，对写信的那位毫无意义。
//!
//! **答形由问形定、帧长可判**：1 字节是答码、8 字节是一个时刻。同一条 `ARM` 路上先到 1 字节
//! （收下了没有），后到 8 字节（到点那一声）。
//!
//! 本文件住**驱动自己那一片目录**，不在 `crates/protocol`：服务面 = 各驱动自己的具体协议
//! （那一条裁定见 `protocol::driver`），而它由驱动与客人**同一份源码**各 `use` 一次。
//! 可共用的只有那两样：**"失败域 ↔ 线上那一格"那张表**（`contract::fail_codes!`）与
//! **成功那一格**（`protocol::OK`）——各家的失败码仍按自己失败域的顺序排。

use super::core::Fail;
use env::{Mark, PieToken};

/// 问那一句的动作码：「现在几点」。
pub const ASK: u8 = 1;

/// 问那一句的动作码：「在 `at` 叫我」。
pub const ARM: u8 = 2;

/// 一个时刻的字节数（u64 LE 纳秒）。
pub const TIME_LEN: usize = 8;

/// 答码的字节数。
pub const CODE_LEN: usize = 1;

/// `ASK` 帧的长度（动作码 + 那一格）。
pub const ASK_LEN: usize = 1 + PieToken::WIDTH;

/// `ARM` 帧的长度（动作码 + 那一格 + 一个时刻）。
pub const ARM_LEN: usize = 1 + PieToken::WIDTH + TIME_LEN;

/// 回信孔的记号：客人每趟铸一枚、借给驱动（**收方按它验那一格**）。
pub const BACK: Mark = Mark::of("rtc-back");

/// 答话那一格：收下了——**全协议那一个"没失败"**（`protocol::OK`），本族不再写第二遍。
pub use protocol::OK;
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

contract::fail_codes! {
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
    /// 「**再过多 long** 叫我」（**相对量**，纳秒）。
    ///
    /// **照实记（为什么是相对量，而不是绝对时刻）**：它从前是绝对时刻，于是客侧要先读一次钟、
    /// 再**猜**一个送达延迟加上去（`AHEAD_NS = 50 ms`）；猜小了驱动答 `Past`，客侧就重问重算
    /// （`ARM_TRIES = 5` 那层环），而那个猜随负载漂（实测一趟常态 ~12.5 ms、重尾到 0.5 s）。
    /// 改成相对量之后 `at = now + after_ns` 由**收帧的人**算——延迟落在谁身上由谁自己承担，
    /// 客侧一个数都不必猜，`Past` 也由构造不可达。
    Arm { after_ns: u64 },
}

/// 编一帧「现在几点」：`[ASK][那一格 8B]`。
///
/// `back` = "我借给你的那枚回信孔**在你表里**是几号"（[`protocol::session::call::lend_out`]
/// 的第二格）——见本文件头注的照实记。
pub fn pack_ask(back: PieToken) -> [u8; ASK_LEN] {
    let mut out = [0u8; ASK_LEN];
    out[0] = ASK;
    out[1..].copy_from_slice(&back.to_bytes());
    out
}

/// 编一帧「再过多 long 叫我」：`[ARM][那一格 8B][after_ns 8B]`——**末格是相对量**（纳秒）。
pub fn pack_arm(back: PieToken, after_ns: u64) -> [u8; ARM_LEN] {
    let mut out = [0u8; ARM_LEN];
    out[0] = ARM;
    out[1..1 + PieToken::WIDTH].copy_from_slice(&back.to_bytes());
    out[1 + PieToken::WIDTH..].copy_from_slice(&after_ns.to_le_bytes());
    out
}

/// 拆一帧问：返 `(那一格, 那一问)`。**不是那个形状就答 `None`**（别人往这扇门推别的东西时，
/// 不猜、不动账）。
///
/// **长度也要对**：`ASK` 只认 `ASK_LEN`、`ARM` 只认 `ARM_LEN`（旧版按"动作码 + 余下都是 at"
/// 解，故长短都进得来；现在那一格也在帧里，长度就是形状的一半）。
pub fn unpack_ask(frame: &[u8]) -> Option<(PieToken, Ask)> {
    let back = PieToken::from_bytes(frame.get(1..)?)?;
    match *frame.first()? {
        ASK if frame.len() == ASK_LEN => Some((back, Ask::Now)),
        ARM if frame.len() == ARM_LEN => {
            let raw: [u8; TIME_LEN] = frame[1 + PieToken::WIDTH..].try_into().ok()?;
            Some((back, Ask::Arm { after_ns: u64::from_le_bytes(raw) }))
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
