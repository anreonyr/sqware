//! rtc::call — **帧形与记号**：两句话、两种答形（**一张字段表就是一处定义**）。
//!
//! ```text
//!   问（客人 → 驱动）   Now  [ASK][那一格 8B]                →  Time    [时刻 8B]   「现在几点」
//!                       Arm  [ARM][那一格 8B][after 8B]      →  Status  [答码 1B]   「再过多 long 叫我」
//!                                                            … 到点那一声 Time [时刻 8B]
//!   回信孔（客人 ↔ 驱动）  记号 `rtc-back` —— 客人**每趟**铸一枚、借给驱动，
//!                          并把"它**在驱动表里**是几号"写进帧
//! ```
//!
//! **四张表、两个方向**：[`Now`] / [`Arm`] 是问的两形，[`Time`] / [`Status`] 是答的两形——偏移与
//! 长度全部由字段宽度求和得出（`env::frame!` 那一处定义），手写的那五枚自由函数
//! （`pack_ask` / `pack_arm` / `unpack_ask` / `pack_time` / `unpack_time`）与那四个长度常量
//! （`ASK_LEN` / `ARM_LEN` / `TIME_LEN` / `CODE_LEN`）一起退场。
//!
//! **两形为什么不合成一张表**：一问的末格只有 `Arm` 有（`Now` 少 8 字节），答的两形长度也不同
//! （1 / 8）——**长度就是形状本身**。故问走 [`Wire::take`] 一处"动作码 ＋ 长度"的判据；答各按
//! 自己那一形收（[`Time`] 恰好 8 字节、[`Status`] 恰好 1 字节）。
//!
//! **照实记（这一族没有 `Union`）**：盟籍那一族的答有三形、客侧同一条路上要认其中几形，故它
//! 按长度分派成一个 `Union`；本族**每一步只有一形**（问时刻 ⇒ 只收 `Time`，约定 ⇒ 只收
//! `Status`）⇒ 长短不对就是读不懂，判据落在**问的人**那一型上，不另立一个读面。
//!
//! **船台只上答那一半**（`protocol::session::slip::Slip`）：答的两侧各拿一枚**裸孔**——驱动
//! `seal(back).load(..).ship()`、客人 `seal(back).land(..)`（到点那一声同）。问那两侧上不去：
//! 客侧推的是**门**（`session::call::push_to`，与 `principal` 的客侧同一手），驱动那侧要
//! **内核盖的发送者印章**（`pull_timeout_from` 那一扫的验），而船台只返报文。故问那两张表
//! **不实现 [`Message`]**——今天没有一处读它（同两族共用的那张 `Query`）。
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
//! **照实记（这一族的帧边角今天仍没有跑着的判据）**：本文件住**驱动自己那一片目录**（见下一
//! 段），而 `programs → protocol → runtime` 在宿主上编不成（`runtime/src/core/tls.rs` 那两行
//! riscv 内联汇编）⇒ 宿主那一侧够不着它。真路只有**人工起一趟**（`cargo image` + `cargo run`
//! ——**照实记**：原先由 `crates/gate` 那扇 `soak` 门自动跑，那台已删。真客人
//! `harness/src/sleeper.rs` 走：`Now`→[`Time`]、
//! `Arm`→[`Status`]`(OK)`、再约一次→[`Status`]`(TAKEN)`、到点→[`Time`]）。**够不着的是畸形帧**
//! ——长短不对 / 动作码不认 / 答话那一格长度不对：判据与手写那版**一字不改**，但它今天仍是
//! "写着的规格"。要让它有跑着的判据，唯一的路是把帧挪进「约」（`contract`），而
//! `contract::driver` 那张表写着"服务面不放本层"——那是翻裁定的一刀，不混在这一条里。
//!
//! 本文件住**驱动自己那一片目录**，不在 `crates/protocol`：服务面 = 各驱动自己的具体协议
//! （那一条裁定见 `protocol::driver`），而它由驱动与客人**同一份源码**各 `use` 一次。
//! 可共用的只有那几样：**帧的骨架**（`env::frame!`）、**报文那一条约定**（[`Message`] 与
//! 船台）、**"失败域 ↔ 线上那一格"那张表**（`contract::fail_codes!`）与**成功那一格**
//! （`protocol::OK`）——各家的失败码仍按自己失败域的顺序排。

use super::core::Fail;
use contract::message::Message;
use env::{Mark, PieToken};

// ── 码 ──────────────────────────────────────────────────────

/// 问那一句的动作码：「现在几点」。
pub const ASK: u8 = 1;

/// 问那一句的动作码：「在 `at` 叫我」。
pub const ARM: u8 = 2;

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
/// 合流——`system::board::frame` 那张表里 `BAD` 在表外、`Denied` 另有 `DENIED` 一格，
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

// ── 一问：两形各一张表 ───────────────────────────────────────

env::frame! {
    /// **问那一形 · 「现在几点」**：动作码 ＋ 那一格。
    ///
    /// 动作码由 [`Now::of`] 钉进来（表那一格是裸字节，是构造那一手保证的）。
    pub struct Now {
        op: u8,
        back: PieToken,
    }
}

env::frame! {
    /// **问那一形 · 「再过多 long 叫我」**：动作码 ＋ 那一格 ＋ **一个相对量**。
    pub struct Arm {
        op: u8,
        back: PieToken,
        after_ns: u64,
    }
}

impl Now {
    /// 编一问：`back` = "我借给你的那枚回信孔**在你表里**是几号"
    /// （`session::call::lend_out` 的第二格）——见本文件头注的照实记。
    pub fn of(back: PieToken) -> Now {
        Now { op: ASK, back }
    }
}

impl Arm {
    /// 编一问：`after_ns` 是**相对量**（纳秒）——绝对时刻由收帧的人算（见 [`Wire::Arm`]）。
    pub fn of(back: PieToken, after_ns: u64) -> Arm {
        Arm {
            op: ARM,
            back,
            after_ns,
        }
    }
}

/// **解出来的一问**——与板 / 树 / 名册 / 盟籍四族同一个名字同一个位置（"解出来的一问"叫 `Wire`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
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

impl Wire {
    /// 解一问：返 `(那一格, 那一问)`。**不是那个形状就答 `None`**（别人往这扇门推别的东西时，
    /// 不猜、不动账、也不回话——那一格读得出来也不答，因为没有可信的"往哪回"可言：动作码不认
    /// 就问不出这一帧该有多长）。
    ///
    /// **长度也要对**：`ASK` 只认 [`Now::LEN`]、`ARM` 只认 [`Arm::LEN`]（旧版按"动作码 + 余下
    /// 都是 at"解，故长短都进得来；现在那一格也在帧里，长度就是形状的一半）。
    pub fn take(bytes: &[u8]) -> Option<(PieToken, Wire)> {
        match *bytes.first()? {
            ASK if bytes.len() == Now::LEN => {
                let ask = Now::fetch(bytes)?;
                Some((ask.back, Wire::Now))
            }
            ARM if bytes.len() == Arm::LEN => {
                let ask = Arm::fetch(bytes)?;
                Some((ask.back, Wire::Arm { after_ns: ask.after_ns }))
            }
            _ => None,
        }
    }
}

// ── 一答：两形各一张表（**只有这两张上船台**）────────────────

env::frame! {
    /// **答那一形 · 一个时刻**：驱动读设备那一刻的纳秒计数（u64 LE）。
    pub struct Time {
        ns: u64,
    }
}

env::frame! {
    /// **答那一形 · 一个答码**：收下了没有（[`OK`] / [`TAKEN`] / [`PAST`] / [`BAD`]）。
    ///
    /// 与板 / 树那两族的 1 字节答**同名同位**（`Status`）：一格状态、没有荷载。
    pub struct Status {
        status: u8,
    }
}

impl Time {
    /// 编一答：一个时刻。
    pub const fn of(ns: u64) -> Time {
        Time { ns }
    }
}

impl Status {
    /// 编一答：一个答码。
    pub const fn of(code: u8) -> Status {
        Status { status: code }
    }
}

impl Message for Time {
    /// 解开之后就是**那个时刻**——读的人不必再念一遍"它叫 `ns`"。
    type In = u64;
    /// 定长一答（[`Time::LEN`]）。
    type Buf = [u8; Time::LEN];
    const EMPTY: Self::Buf = [0u8; Time::LEN];

    /// 表那一手 `store_in`（写更大的缓冲、返长度）——正是这一手要的；表上那枚**同名**的 `store`
    /// 要的是定长数组、返 `()`，两回事。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Time::store_in(self, out)
    }

    /// **恰好 8 字节**：表那一手只要求"够长"，而这一形今天的判据是"长短都不认"——长一字节也是
    /// 读不懂（旧 `unpack_time` 那一句 `frame.try_into()` 逐字就是这个判据）。
    ///
    /// `Time::fetch` 在这里指的是**表那一手**：两枚同名，靠语言那条"固有 impl 优先于 trait"分得
    /// 开，不是递归（同 `contract::frame` 里 `Reply` 那一处）。
    fn fetch(bytes: &[u8]) -> Option<u64> {
        if bytes.len() != Time::LEN {
            return None;
        }
        Some(Time::fetch(bytes)?.ns)
    }
}

impl Message for Status {
    /// 解开之后就是**那一格答码**。
    type In = u8;
    type Buf = [u8; Status::LEN];
    const EMPTY: Self::Buf = [0u8; Status::LEN];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Status::store_in(self, out)
    }

    /// **恰好 1 字节**（旧那一句 `n == CODE_LEN` 逐字就是这个判据）。
    fn fetch(bytes: &[u8]) -> Option<u8> {
        if bytes.len() != Status::LEN {
            return None;
        }
        Some(Status::fetch(bytes)?.status)
    }
}
