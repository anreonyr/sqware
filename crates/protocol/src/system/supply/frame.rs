//! supply::frame — **形**（线上形状）：单子（[`Order`]）与回单（[`Reply`]）、上限与状态码
//!
//! **照实记（这一份从前是什么样）**：单子与回单各有一对**自由函数**（`pack_order` /
//! `unpack_order`、`pack_reply` / `unpack_reply`）、两个**借字节的视图**（`Order<'a>` /
//! `Reply<'a>`）、一处手算的帧头（`HEAD_LEN`）——"多长、怎么写、怎么读"散在三处。今天收成报那一层
//! 那两样：**一张字段表**（`#[derive(env::Frame)]` 求长）＋ **一个 `impl Message`**（编解一处）；
//! 那一段重复走 `#[frame(count = n, fill = …)]`（`store_tail` / `fetch_tail` 由宏接线）。
//!
//! **照实记（`OrderHead` / `ReplyHead` 并回本表）**：那两枚头从前只为给尾巴算偏移而单立，
//! 而条数在内存里又是同一个数（`usize`）——今天一格就是那一帧，条数那一格是线上那一格。
//!
//! **照实记（这一族两端都上类型化手柄——上一版这里写反了）**：
//!
//! · **客侧**（`protocol::system::supply::client::draw`）：发走 `Sender::<Order>`、收走
//!   `Receiver::<Reply>`。它那两句判据仍分得开——"期限内没等到" ⇒ `Local`、"收下来解不动"
//!   ⇒ `Bad`——靠的是 `Receiver::recv` 那三格失败（`Mail` 那一族的忙/死/拒 ＋ `Unread`）。
//! · **服务侧**（`programs::root::supply::server`）：收帧走同一手（"先探活、再解题"那两格照旧），
//!   回单走 `Sender::<Reply>` 那一手；那头没齐（没有写端）时不发，与从前同一格。
//!
//! ⇒ 编解一处（表 ＋ `Message`）、收发一处（手柄），运输只剩"泊位就是那条路"这一件。
//!
//! 正文见 `protocol` 那一侧的 `system/supply/mod.rs`（**分批搬家的中途**：正文还没过来）。

use env::TaskId;
use env::Pair;

use crate::message::Message;

// ── 失败域（原先住 `core.rs`：残枝那一刀并进来）────────────────

/// 领不到（或供不成）的**五种**，对应"调用方接下来该干什么"。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fail {
    /// 本地失败：单子装不下 / 这条泊位没有写端 / 期限内没等到。
    Local,
    /// 账里没这个名字。
    Unknown,
    /// 授不出（越权 / 对端不在 / 对端表满）。
    Denied,
    /// 备不下（条数越界 / 缓冲不够）。
    Full,
    /// 帧读不懂。
    Bad,
}

/// 引导域↔编排域那条泊位的名字：**两侧同一个**（泊位自己的坐标，不进报文）。
pub const BOOT: &str = "boot";

// **词汇住 `env::supply`**（装配表要宿主侧也读得到，见那一处头注）：本处只转发，
// **调用点一行没改**。
pub use env::supply::{Kind, WANT_LEN, Want};

/// 单子的操作码。今天只有"供"这一枚——留着这一格，是为加动作时不必改帧的布局。
pub const OP_SUPPLY: u8 = 1;

/// 一条单子最多要五样（今天的单子四样）。
pub const WANT_MAX: usize = 5;

/// 单子 / 回单的定长缓冲：**最长那一形**（头 ＋ 上界那么多条，同一张表求和）。
///
/// **不是线格式的上限**：孔不预设上限（见 `env::fid::PieCall::UnsealHole`），这两个数是本侧选
/// "一帧一单、不流式"的结果。
pub const ORDER_CAP: usize = Order::LEN;
pub const REPLY_CAP: usize = Reply::LEN;

/// 成功那一格：**全协议同一个号**——定义在 `protocol/src/fail_codes.rs`（`fail_codes!` 的第二个参数就是它），
/// 本族只把它转出来。
pub use crate::fail_codes::OK;

/// 回单的状态码（与 operator / system::board 的码表同族）。
pub const UNKNOWN: u8 = 1;
pub const DENIED: u8 = 2;
pub const FULL: u8 = 3;
pub const BAD: u8 = 4;

// ── 一单（问）───────────────────────────────────────────────

/// **一张单子**：哪一动作（今天只有 [`OP_SUPPLY`]）＋ 条数 ＋ **给谁** ＋ 至多 [`WANT_MAX`] 条。
///
/// 条数是**声明**：与后面那一段绑死（读的人两边对不上就是读不懂）。这一族一问只有这一形，
/// 故不留"未完"那一格（对照 coalition 那扇窗：盟籍没有上限）。
///
/// **照实记（头那一枚并进来了）**：它从前分成两处——内存里没有 `op`，另立一枚
/// `OrderHead { op, count, who }` 在编的那一手现拼；derive 认"一段重复"之后**一格就是那一帧**，
/// `op` 由 [`Order::of`] 写、由解码那一手校。
#[derive(env::Frame, Clone, Copy)]
pub struct Order {
    op: u8,
    n: u8,
    who: TaskId,
    #[frame(count = n, fill = Want::NONE)]
    wants: [Want; WANT_MAX],
}

/// **线上一个字节都不许动**：这两形照本节那两张表钉住（`[码 1B][条数 1B][…条]`）。
const _: () = assert!(
    Order::LEN
        == 1 + 1 + <TaskId as env::wire::Field>::WIDTH + WANT_LEN * WANT_MAX
);
const _: () = assert!(Reply::LEN == 1 + 1 + env::PAIR_LEN * WANT_MAX);

impl Order {
    /// 起一张单子。**条数越界 ⇒ `None`**（调用方按本地失败处置）。
    ///
    /// **照实记（"缓冲不够"那一格退场）**：从前 `pack_order` 还答一格"给的那只缓冲装不下"——
    /// 今天缓冲就是这一族最长那一只（[`Message::Buf`]），装不下**不可表达**，故那一格没了。
    pub fn of(who: TaskId, wants: &[Want]) -> Option<Order> {
        let n = wants.len();
        if n > WANT_MAX {
            return None;
        }
        let mut held = [Want::NONE; WANT_MAX];
        held.get_mut(..n)?.copy_from_slice(wants);
        Some(Order {
            op: OP_SUPPLY,
            n: n as u8,
            who,
            wants: held,
        })
    }

    /// 这条单子是**给谁**的（那一格过线的号）。
    pub fn who(&self) -> TaskId {
        self.who
    }

    /// 几条。
    pub fn len(&self) -> usize {
        self.n as usize
    }

    /// 第 `i` 条（越界 ⇒ `None`）。
    pub fn want(&self, i: usize) -> Option<Want> {
        (i < self.len()).then(|| self.wants[i])
    }
}

impl Message for Order {
    /// **写法与读法是同一个**：这一族一问只有一形。
    type In = Order;
    type Buf = [u8; ORDER_CAP];
    const EMPTY: Self::Buf = [0u8; ORDER_CAP];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// 解一张单子：`op` 不对 / 条数越界 / **不够长** ⇒ `None`（不猜、不崩）。
    ///
    /// **照实记（"够长"就是问那一形的判据）**：从前 `unpack_order` 只要求
    /// `len >= 头 ＋ n × 32`——长出来那几字节**不算**读不懂；而**回单**那一形要求**恰好**
    /// （见 [`Reply`] 的 `fetch`）。两条都是旧判据，照抄，没改；这两句是**本族的**，derive 不替它判。
    fn fetch(bytes: &[u8]) -> Option<Order> {
        let (order, _) = Order::fetch_at(bytes, 0)?;
        (order.op == OP_SUPPLY).then_some(order)
    }
}

// ── 一答（回单）─────────────────────────────────────────────

/// **一张回单**：答话那一格（[`OK`] / [`UNKNOWN`] / [`DENIED`] / [`FULL`] / [`BAD`]）＋ 条数
/// ＋ 至多 [`WANT_MAX`] 条记录（坐标 ＋ 号）。
///
/// **照实记（记录为什么是 [`Pair`] 而不是字节）**：编那一侧手上就是 `Pair`（`port::ship` 交回
/// 一枚号，见发货那一侧），读那一侧手上是字节——`Pair` 的那两个 `Field` 手是两者之间**唯一**
/// 那一处（`repr(C)`、尺寸编译期锁死）。
///
/// **照实记（头那一枚并进来了）**：它从前分成两处——内存里 `len: usize`、线上另立一枚
/// `ReplyHead { code, count }`；derive 认"一段重复"之后一格就是那一帧。
#[derive(env::Frame, Clone, Copy)]
pub struct Reply {
    code: u8,
    n: u8,
    #[frame(count = n, fill = Pair::NONE)]
    pairs: [Pair; WANT_MAX],
}

impl Reply {
    /// 编一张回单。**条数越界 ⇒ `None`**。
    pub fn of(code: u8, records: &[Pair]) -> Option<Reply> {
        let n = records.len();
        if n > WANT_MAX {
            return None;
        }
        let mut held = [Pair::NONE; WANT_MAX];
        held.get_mut(..n)?.copy_from_slice(records);
        Some(Reply {
            code,
            n: n as u8,
            pairs: held,
        })
    }

    /// 答话那一格（[`OK`] / [`UNKNOWN`] / [`DENIED`] / [`FULL`] / [`BAD`]）。
    pub fn code(&self) -> u8 {
        self.code
    }

    /// 记录那一段（整条记录，`PAIR_LEN` 步长）。
    pub fn records(&self) -> &[Pair] {
        self.pairs.get(..self.n as usize).unwrap_or(&[])
    }
}

impl Message for Reply {
    /// **写法与读法是同一个**：这一族一答只有一形。
    type In = Reply;
    type Buf = [u8; REPLY_CAP];
    const EMPTY: Self::Buf = [0u8; REPLY_CAP];

    fn store(&self, out: &mut [u8]) -> Option<usize> {
        self.store_at(out, 0)
    }

    /// 解一张回单：条数越界 / **帧长与条数对不上** ⇒ `None`（不猜、不崩）。
    ///
    /// **照实记（"恰好"就是答那一形的判据）**：从前 `unpack_reply` 要的是
    /// `len == 2 ＋ n × PAIR_LEN`——**多一字节也是读不懂**（与问那一形的"够长"相对，见上）。
    /// 这条判据是**本族的**，derive 不替它判。
    fn fetch(bytes: &[u8]) -> Option<Reply> {
        let (reply, at) = Reply::fetch_at(bytes, 0)?;
        (at == bytes.len()).then_some(reply)
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
