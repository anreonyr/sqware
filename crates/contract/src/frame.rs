//! **定长一问一答**那一族帧的骨架 —— `principal` 与 `coalition` **同形的那一份**。
//!
//! ```text
//!   Query  [0] op  [1..9] a  [9..17] b  [17..25] back    25（表求和：`Query::LEN`）
//!   Reply  [0] status  [1] flag  [2..10] a               10（表求和：`Reply::LEN`）
//! ```
//!
//! `a` / `b` 两格的**意义由动作码定**（`RESOLVE`/`DERIVE`/`SIRE` 只填 `a`，`HEIR` 两格都填）；
//! 答话定长，故两侧都不用攒缓冲、也不用问长度。
//!
//! **`back` 那一格是"往哪回"**（末尾 8 字节）：客侧每趟铸一枚回信孔借给对端，
//! [`Query`] 收的那一格就是**它在对端表里的号**——对端据此一次 `Reserve` 验出来，
//! 不必再扫自己的表按"谁给的 ＋ 记号"去找。这一格的名字两侧各按自己的角色念：
//! 客侧叫 **seed**（"种在你表里的那一枚"）、服务端叫 `back`，[`Query`] 的 `back` 那一格按服务端
//! 那一侧叫（读它的是服务端）。照实记那一刀的账见 [`Query`] 与 `session::call::lend_out`。
//!
//! **这里只放"同形"的那几手**：两族的**码**（动作码与答话码）各按自己的失败域排、记号各是各的
//! （`principal-back` / `coalition-back`）、`fail_codes!` 表各装各的失败域——那些都留在各族自己的
//! `frame.rs`。**身体搬到这里，两处只留各自的名字**（同 `session::call` 那条纪律）。
//!
//! **两样不在这里，各有各的理由**：`reply_present`（"有没有一条号"）只有 principal 用 ⇒ **一位
//! 用家不搬**；`SeqHead` / `Union`（**窗**那一档：coalition 那三种答形的后两种）只有那一家用 ⇒
//! 留在 `system::coalition::frame`——operator 的"一条 pane 本来就有顶"不需要"未完"那一格，
//! 故窗不是这一族的共性。
//!
//! 本文件是**协议层**的东西，与 `id.rs` / `fail_codes.rs` 同一种编法。

use crate::fail_codes::OK;
use crate::id::Id;
use crate::message::Message;
use env::PieToken;

// ── 一问那一形 ──────────────────────────────────────────────

/// **一问那一形**（principal 与 coalition **同形**）：动作码 ＋ 两个 8 字节的号 ＋
/// **回信孔那一格**。
///
/// `a` / `b` 两格的**意义由动作码定**（各族那枚 `Req` 说它这一条有几格）；`back` 是**运输**
/// 那一格（往哪回），不是动作的荷载——故它排最后，谁都不许把它当第三个号使。
///
/// **照实记（`back` 那一格为什么在帧里）**：从前它不在——客侧 `lend` 把 `port::ship` 的第二格
/// （`to.seed()`，就是"我给你的那一枚在你表里是几号"）**扔了**，于是服务端只能**扫自己的表**
/// 按"谁给的 ＋ 记号"把那一枚认回来（`session::call::find`）。那一扫是每趟请求一遍全表，
/// 而 `Collect` 每枚还要算一次 `vestor`（吃全世界快照）——读数见
/// `programs/src/driver/rtc/main.rs` 与提交 `444d1f3` / `b58fda4`。
/// 把它放进帧里之后，服务端**一次 `Reserve` 就验完**（判据一字未改：谁开的 ＋ 记号）。
///
/// **照实记（"`ASK_LEN = 17`"那一句是假的）**：本文件、`principal/frame.rs`、
/// `coalition/frame.rs` 与两族的 `mod.rs` 原先都写"一问 17 字节"——那是 `back` 那一格
/// **落地之前**抄的，此后它一直是 `1 + 8 + 8 + 8 = 25`（`pack_ask` 写满 25、`unpack_ask`
/// 要 25）。今天这个数**一处都不写**（表求和），那几处假的也一并改真。
#[derive(env::Frame)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Query {
    pub op: u8,
    pub a: u64,
    pub b: u64,
    pub back: PieToken,
}

// **编 / 解**：两族各自的 `Req` / `Wire` 用表自己那两手（`store` / `fetch`）。
//
// **照实记（"这一族两侧都不是孔"那一句是假的）**：本处原写着"客侧推的是**入口**、持册者收的是
// **一页**，故这一族没有 `Slip` 的用家"——**理由错了**：客侧推的那一枚入口就是一枚孔（`push_to`
// 与 `Slip::ship` 是同一手，板、树两族的客侧正是这么推的），答话那一侧两侧也都是孔（客侧借出的
// 那一枚、持册者往它推）。问话这一侧用不上船台的**真原因在收的那一头**：持册者要的是**内核盖
// 章的发送者**（`pull_timeout_from` 那两格——"谁开的 ＋ 记号"就是这一族的钥匙），而船台只返报
// 文。
//
// **照实记（"客侧一问也能上船台、3a 是个漏"——这一句收回来）**：照着上面那条，"客侧那一枚确实
// 是孔 ⇒ 没换是个漏"我写过。细看**不是漏**：`Slip` 要一整个 `Message`（`In` / `fetch` /
// `EMPTY`），而 [`Query`] 是**两族共用的表**、两族的读面不同（`principal::Wire` 与
// `coalition::Wire` 是两种类型）⇒ **立不出一个共用的 `In`**；客侧今天那两手（表上的 `store` ＋
// `session::call::push_to`）已经是最小形状——`Slip` 只多包一层推，收益为零、代价是一张用不上的
// 读面。故这一族**问话那一侧不上船台，答话那一侧才上**。

// ── 一答那一形 ──────────────────────────────────────────────

/// **一答那一形**（两族同形）：状态 ＋ 有没有 ＋ 一枚号。
///
/// `a` 那一格是**裸的 8 字节小端**（[`Id::to_bytes`] 就是它，与 `Field for u64` 同一条
/// 口径）：principal 的 `DERIVE` / `SIRE` / `RESOLVE` 与 coalition 的 `FOUND` 都填这一格。
///
/// **照实记（`flag` 那一格：`== 1` 变成严格 0 / 1）**：换表之前这一格由 `unpack_reply` 交成**裸
/// 字节**，读法是**调用方**各写的那一句 `present == 1`。换表那一刀把读法交给 `Field for bool`，
/// 而当**那一格还是 `!= 0`** 时，畸形的 `2` 会读成"是"——故这一条账当时记着"要严格就把
/// '这一格只许 0 / 1'并进判据"。**今天严格收在 [`env::wire::Field`] 一处了**（`bool` 那一格只
/// 认 0 / 1）：合法帧逐字不变，畸形的 `2` 整个读不懂（`fetch` 答 `None`），本文件不再另判。
#[derive(env::Frame)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reply {
    pub status: u8,
    pub flag: bool,
    pub a: u64,
}

impl Reply {
    /// 编一答：只有状态那一格（失败，或读不懂）。
    pub const fn status(code: u8) -> Reply {
        Reply {
            status: code,
            flag: false,
            a: 0,
        }
    }

    /// 编一答：`OK` + 是 / 不是（principal 的 `HEIR`、coalition 的 `AMID`）。
    pub const fn yes(yes: bool) -> Reply {
        Reply {
            status: OK,
            flag: yes,
            a: 0,
        }
    }

    /// 编一答：`OK` + 一枚号。
    ///
    /// **号的类型是泛型**（[`Id`]）：两族的号是两种类型，而"填进那一格"这件事一模一样。
    ///
    /// **`flag` 那一格不用**：这一路的答案**必有**号（零号也是合法答案）——"有没有"是另一条路
    /// （`principal::frame::reply_present`，只 principal 有）。
    pub fn value<T: Id>(at: T) -> Reply {
        Reply {
            status: OK,
            flag: false,
            a: at.get() as u64,
        }
    }
}

impl Message for Reply {
    /// **写法与读法是同一个**：这一形三格俱全，读的人不必再问"我问的是哪一条"。
    type In = Reply;
    /// 定长一答（[`Reply::LEN`]）。
    type Buf = [u8; Reply::LEN];
    const EMPTY: Self::Buf = [0u8; Reply::LEN];

    /// 表那一手 `store_in`（写更大的缓冲、返长度）——正是这一手要的；表上那枚**同名**的 `store`
    /// 要的是定长数组、返 `()`，两回事。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Reply::store_in(self, out)
    }

    /// **恰好 10 字节**：表那一手只要求"够长"，而这一形今天的判据是"长短都不认"——长一字节也是
    /// 读不懂（同 `board` 那一族那条照实记）。
    ///
    /// `Reply::fetch` 在这里指的是**表那一手**：两枚同名，靠语言那条"固有 impl 优先于 trait"分得
    /// 开，不是递归。判据是宿主的往返用例（10 说得回来、9 与 11 都读不懂）。
    fn fetch(bytes: &[u8]) -> Option<Reply> {
        if bytes.len() != Reply::LEN {
            return None;
        }
        Reply::fetch(bytes)
    }
}
