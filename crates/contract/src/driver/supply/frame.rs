//! supply::frame — **形**（线上形状）：单子（[`Order`]）与回单（[`Reply`]）、上限与状态码
//!
//! **照实记（这一份从前是什么样）**：单子与回单各有一对**自由函数**（`pack_order` /
//! `unpack_order`、`pack_reply` / `unpack_reply`）、两个**借字节的视图**（`Order<'a>` /
//! `Reply<'a>`）、一处手算的帧头（`HEAD_LEN`）——"多长、怎么写、怎么读"散在三处。今天收成报那一层
//! 那两样：**一张头表**（`env::frame!` 求长）＋ **一个 `impl Message`**（编解一处）；尾巴那两段走
//! `env::wire::{store_tail, fetch_tail}`。
//!
//! **照实记（这一族两端都上了船台——上一版这里写反了）**：
//!
//! · **客侧**（`protocol::driver::supply::client::draw`）：**搬进 `protocol` 之后**才用得上船台
//!   （那一层同时看得见"孔"与"报"）。它那两句判据仍分得开——"期限内没等到" ⇒ `Local`、
//!   "收下来解不动" ⇒ `Bad`——靠的是 `Slip::land` 那两格失败（`Land`：没收到 / 解不动）。
//! · **服务侧**（`programs::root::supply::server`）：收帧走同一手（"先探活、再解题"那两格照旧），
//!   回单走 `Slip::<Reply>` 那一手；泊位那头没齐时不发（与从前 `Pier::post` 同一格）。
//!
//! ⇒ 编解一处（表 ＋ `Message`）、收发一处（船台），运输只剩"泊位就是那条路"这一件。
//!
//! 正文见 `protocol` 那一侧的 `driver/supply/mod.rs`（**分批搬家的中途**：正文还没过来）。

use env::TaskId;
use plan::{PAIR_LEN, Pair};

use crate::message::Message;

use super::core::Fail;

/// 引导域↔编排域那条泊位的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const BOOT: &str = "boot";

// **词汇搬去 `plan` 了**（装配单要宿主侧也读得到，见 `plan::supply` 头注）：本处只转发，
// **调用点一行没改**。
pub use plan::supply::{At, Kind, Need, WANT_LEN, Want, class_block};

/// 单子的操作码。今天只有"供"这一枚——留着这一格，是为加动作时不必改帧的布局。
pub const OP_SUPPLY: u8 = 1;

/// 一条单子最多要五样（今天的单子四样）。
pub const WANT_MAX: usize = 5;

/// 单子 / 回单的定长缓冲：**头 ＋ 上界那么多条**（一处求和）。
///
/// **不是线格式的上限**：孔不预设上限（见 `env::fid::PieCall::UnsealHole`），这两个数是本侧选
/// "一帧一单、不流式"的结果。
pub const ORDER_CAP: usize = OrderHead::LEN + WANT_LEN * WANT_MAX;
pub const REPLY_CAP: usize = ReplyHead::LEN + PAIR_LEN * WANT_MAX;

/// 成功那一格：**全协议同一个号**——定义在 `contract/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 回单的状态码（与 operator / system::board 的码表同族）。
pub const UNKNOWN: u8 = 1;
pub const DENIED: u8 = 2;
pub const FULL: u8 = 3;
pub const BAD: u8 = 4;

// ── 一单（问）───────────────────────────────────────────────

env::frame! {
    /// 单子那三格头：**哪一动作**（今天只有 [`OP_SUPPLY`]）＋ 条数 ＋ **给谁**。
    ///
    /// 条数是**声明**：与后面那一段绑死（读的人两边对不上就是读不懂）。这一族一问只有这一形，
    /// 故不留"未完"那一格（对照 coalition 那扇窗：盟籍没有上限）。
    pub struct OrderHead {
        op: u8,
        count: u8,
        who: TaskId,
    }
}

/// **一张单子**：给谁 ＋ 至多 [`WANT_MAX`] 条（尾巴走 [`env::wire::store_tail`]）。
#[derive(Clone, Copy)]
pub struct Order {
    who: TaskId,
    len: usize,
    wants: [Want; WANT_MAX],
}

impl Order {
    /// 起一张单子。**条数越界 ⇒ `None`**（调用方按本地失败处置）。
    ///
    /// **照实记（"缓冲不够"那一格退场）**：从前 `pack_order` 还答一格"给的那只缓冲装不下"——
    /// 今天缓冲就是这一族最长那一只（[`Message::Buf`]），装不下**不可表达**，故那一格没了。
    pub fn of(who: TaskId, wants: &[Want]) -> Option<Order> {
        let len = wants.len();
        if len > WANT_MAX {
            return None;
        }
        let mut held = [Want::NONE; WANT_MAX];
        held.get_mut(..len)?.copy_from_slice(wants);
        Some(Order {
            who,
            len,
            wants: held,
        })
    }

    /// 这条单子是**给谁**的（那一格过线的号）。
    pub fn who(&self) -> TaskId {
        self.who
    }

    /// 几条。
    pub fn len(&self) -> usize {
        self.len
    }

    /// 第 `i` 条（越界 ⇒ `None`）。
    pub fn want(&self, i: usize) -> Option<Want> {
        (i < self.len).then(|| self.wants[i])
    }
}

impl Message for Order {
    /// **写法与读法是同一个**：这一族一问只有一形。
    type In = Order;
    type Buf = [u8; ORDER_CAP];
    const EMPTY: Self::Buf = [0u8; ORDER_CAP];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        let head = OrderHead {
            op: OP_SUPPLY,
            count: self.len as u8,
            who: self.who,
        };
        let at = head.store_in(out)?;
        env::wire::store_tail(out, at, &self.wants[..self.len])
    }

    /// 解一张单子：`op` 不对 / 条数越界 / **不够长** ⇒ `None`（不猜、不崩）。
    ///
    /// **照实记（"够长"就是问那一形的判据）**：从前 `unpack_order` 只要求
    /// `len >= 头 ＋ n × 32`——长出来那几字节**不算**读不懂；而**回单**那一形要求**恰好**
    /// （见 [`Reply`] 的 `fetch`）。两条都是旧判据，照抄，没改。
    fn fetch(bytes: &[u8]) -> Option<Order> {
        let head = OrderHead::fetch(bytes)?;
        if head.op != OP_SUPPLY {
            return None;
        }
        let len = head.count as usize;
        if len > WANT_MAX {
            return None;
        }
        let body = bytes.get(OrderHead::LEN..)?;
        if body.len() < len * WANT_LEN {
            return None;
        }
        let mut wants = [Want::NONE; WANT_MAX];
        env::wire::fetch_tail(body, 0, &mut wants[..len])?;
        Some(Order {
            who: head.who,
            len,
            wants,
        })
    }
}

// ── 一答（回单）─────────────────────────────────────────────

env::frame! {
    /// 回单那头两格：**答话那一格**（[`OK`] / [`UNKNOWN`] / [`DENIED`] / [`FULL`] / [`BAD`]）
    /// ＋ 条数。
    pub struct ReplyHead {
        code: u8,
        count: u8,
    }
}

/// **一张回单**：答话那一格 ＋ 至多 [`WANT_MAX`] 条记录（坐标 ＋ 号）。
///
/// **照实记（记录为什么是 [`Pair`] 而不是字节）**：编那一侧手上就是 `Pair`（`port::ship` 交回
/// 一枚号，见发货那一侧），读那一侧手上是字节——`Pair` 的那两个 `Field` 手是两者之间**唯一**
/// 那一处（`repr(C)`、尺寸编译期锁死）。
#[derive(Clone, Copy)]
pub struct Reply {
    code: u8,
    len: usize,
    pairs: [Pair; WANT_MAX],
}

impl Reply {
    /// 编一张回单。**条数越界 ⇒ `None`**。
    pub fn of(code: u8, records: &[Pair]) -> Option<Reply> {
        let len = records.len();
        if len > WANT_MAX {
            return None;
        }
        let mut held = [Pair::NONE; WANT_MAX];
        held.get_mut(..len)?.copy_from_slice(records);
        Some(Reply {
            code,
            len,
            pairs: held,
        })
    }

    /// 答话那一格（[`OK`] / [`UNKNOWN`] / [`DENIED`] / [`FULL`] / [`BAD`]）。
    pub fn code(&self) -> u8 {
        self.code
    }

    /// 记录那一段（整条记录，`PAIR_LEN` 步长）。
    pub fn records(&self) -> &[Pair] {
        self.pairs.get(..self.len).unwrap_or(&[])
    }
}

impl Message for Reply {
    /// **写法与读法是同一个**：这一族一答只有一形。
    type In = Reply;
    type Buf = [u8; REPLY_CAP];
    const EMPTY: Self::Buf = [0u8; REPLY_CAP];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        let head = ReplyHead {
            code: self.code,
            count: self.len as u8,
        };
        let at = head.store_in(out)?;
        env::wire::store_tail(out, at, self.records())
    }

    /// 解一张回单：条数越界 / **帧长与条数对不上** ⇒ `None`（不猜、不崩）。
    ///
    /// **照实记（"恰好"就是答那一形的判据）**：从前 `unpack_reply` 要的是
    /// `len == 2 ＋ n × PAIR_LEN`——**多一字节也是读不懂**（与问那一形的"够长"相对，见上）。
    fn fetch(bytes: &[u8]) -> Option<Reply> {
        let head = ReplyHead::fetch(bytes)?;
        let len = head.count as usize;
        if len > WANT_MAX {
            return None;
        }
        let body = bytes.get(ReplyHead::LEN..)?;
        if body.len() != len * PAIR_LEN {
            return None;
        }
        let mut pairs = [Pair::NONE; WANT_MAX];
        env::wire::fetch_tail(body, 0, &mut pairs[..len])?;
        Some(Reply {
            code: head.code,
            len,
            pairs,
        })
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

crate::fail_codes! {
    /// 本地失败域 → 线上状态码。`None`（没失败）⇒ `OK`——与上面那个 [`code_to_fail`] 的
    /// `OK ⇒ None` 正好是同一格的两侧读法。
    ///
    /// **本表不是双射**（`Local` 与 `Bad` 同归 `BAD`），故宏**不给反向**：反向由人写在上面，
    /// 并注明它反不回来。
    lossy Fail; OK;
    Fail::Local | Fail::Bad => BAD,
    Fail::Unknown => UNKNOWN,
    Fail::Denied => DENIED,
    Fail::Full => FULL,
}
