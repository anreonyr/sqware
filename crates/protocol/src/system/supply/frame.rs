//! supply::frame — **形**（线上形状）：单子（[`Order`]）与回单（[`Reply`]）、上限与状态码

use env::Pair;
use env::TaskId;

use crate::message::Message;

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

pub const OP_SUPPLY: u8 = 1;

/// 一条单子最多要五样（今天的单子四样）。
pub const WANT_MAX: usize = 5;

/// 单子 / 回单的定长缓冲：**最长那一形**（头 ＋ 上界那么多条，同一张表求和）。
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

/// **一张单子**：哪一动作（今天只有 [`OP_SUPPLY`]）＋ 条数 ＋ **给谁** ＋ 至多 [`WANT_MAX`] 条。
/// 条数是**声明**：与后面那一段绑死（读的人两边对不上就是读不懂）。这一族一问只有这一形，
/// 故不留"未完"那一格（对照 coalition 那扇窗：盟籍没有上限）。
#[derive(env::Frame, Clone, Copy)]
pub struct Order {
    op: u8,
    n: u8,
    who: TaskId,
    #[frame(count = n, fill = Want::NONE)]
    wants: [Want; WANT_MAX],
}

/// **线上一个字节都不许动**：这两形照本节那两张表钉住（`[码 1B][条数 1B][…条]`）。
const _: () =
    assert!(Order::LEN == 1 + 1 + <TaskId as env::wire::Field>::WIDTH + WANT_LEN * WANT_MAX);
const _: () = assert!(Reply::LEN == 1 + 1 + env::PAIR_LEN * WANT_MAX);

impl Order {
    /// 起一张单子。**条数越界 ⇒ `None`**（调用方按本地失败处置）。
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
    fn fetch(bytes: &[u8]) -> Option<Order> {
        let (order, _) = Order::fetch_at(bytes, 0)?;
        (order.op == OP_SUPPLY).then_some(order)
    }
}

/// **一张回单**：答话那一格（[`OK`] / [`UNKNOWN`] / [`DENIED`] / [`FULL`] / [`BAD`]）＋ 条数
/// ＋ 至多 [`WANT_MAX`] 条记录（坐标 ＋ 号）。
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
    fn fetch(bytes: &[u8]) -> Option<Reply> {
        let (reply, at) = Reply::fetch_at(bytes, 0)?;
        (at == bytes.len()).then_some(reply)
    }
}

/// 线上状态码 → 本地失败域。`OK` 不是失败，故返 `None`。
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
    /// **本表不是双射**（`Local` 与 `Bad` 同归 `BAD`），故宏**不给反向**：反向由人写在上面，
    /// 并注明它反不回来。
    lossy Fail; OK;
    Fail::Local | Fail::Bad => BAD,
    Fail::Unknown => UNKNOWN,
    Fail::Denied => DENIED,
    Fail::Full => FULL,
}
