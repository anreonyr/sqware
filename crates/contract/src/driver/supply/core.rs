//! supply::core — **失败域**：领不到 / 供不成的五种（不是「内核哪一步坏了」，是「调用方接下来该干什么」）
//!
//! 正文见 `protocol` 那一侧的 `driver/supply/mod.rs`；记号、帧与上限见 [`frame`](crate::driver::supply::frame)。

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
