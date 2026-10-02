//! 定长一问一答那一族帧的骨架 —— principal 与 coalition 同形的那一份。
//! `a` / `b` 两格的**意义由动作码定**（`RESOLVE`/`DERIVE`/`SIRE` 只填 `a`，`HEIR` 两格都填）；
//! 答话定长，故两侧都不用攒缓冲、也不用问长度。
//! **`back` 那一格是"往哪回"**（末尾 8 字节）：客侧每趟铸一枚回信孔借给对端，
//! Query 收的那一格就是**它在对端表里的号**——对端据此一次 `Reserve` 验出来，

use crate::wire::OK;
use crate::wire::id::Id;
use crate::wire::message::Message;
use env::PieToken;
use env::wire::Span as _;

/// **一问那一形**（principal 与 coalition **同形**）：动作码 ＋ 两个 8 字节的号 ＋
/// **回信孔那一格**
/// `a` / `b` 两格的**意义由动作码定**（各族那枚 `Req` 说它这一条有几格）；`back` 是**运输**
/// 那一格（往哪回），不是动作的荷载——故它排最后，谁都不许把它当第三个号使
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Query {
    pub op: u8,
    pub a: u64,
    pub b: u64,
    pub back: PieToken,
}

// **编 / 解**：两族各自的 `Req` / `Wire` 用表自己那两手（`store` / `fetch`）。

/// **一答那一形**（两族同形）：状态 ＋ 有没有 ＋ 一枚号
/// `a` 那一格是**裸的 8 字节小端**（Id::to_bytes 就是它，与 `Field for u64` 同一条
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reply {
    pub status: u8,
    pub flag: bool,
    pub a: u64,
}

impl Reply {
    /// 编一答：只有状态那一格（失败，或读不懂）
    pub const fn status(code: u8) -> Reply {
        Reply {
            status: code,
            flag: false,
            a: 0,
        }
    }

    /// 编一答：`OK` + 是 / 不是（principal 的 `HEIR`、coalition 的 `AMID`）
    pub const fn yes(yes: bool) -> Reply {
        Reply {
            status: OK,
            flag: yes,
            a: 0,
        }
    }

    /// 编一答：`OK` + 一枚号
    /// **号的类型是泛型**（Id）：两族的号是两种类型，而"填进那一格"这件事一模一样
    /// **`flag` 那一格不用**：这一路的答案**必有**号（零号也是合法答案）——"有没有"是另一条路
    /// （principal::frame::reply_present，只 principal 有）
    pub fn value<T: Id>(at: T) -> Reply {
        Reply {
            status: OK,
            flag: false,
            a: at.get() as u64,
        }
    }
}

impl Message for Reply {
    /// **写法与读法是同一个**：这一形三格俱全，读的人不必再问"我问的是哪一条"
    type In = Reply;
    /// 定长一答（Reply::LEN）
    type Buf = [u8; Reply::LEN];
    const EMPTY: Self::Buf = [0u8; Reply::LEN];

    /// 要的是定长数组、返 `()`，两回事
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        Reply::store_at(self, out, 0)
    }

    fn fetch(bytes: &[u8]) -> Option<Reply> {
        if bytes.len() != Reply::LEN {
            return None;
        }
        Reply::fetch_at(bytes, 0).map(|one| one.0)
    }
}
