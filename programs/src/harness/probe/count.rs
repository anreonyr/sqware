//! 探针共用的量具：**数一块窗格底下到齐没有**。
//!
//! 各族"该有几枚"由**各自的** `Grant::ALL.len()` 说（一枚 `Grant` = 一枚门牌 = 一格），
//! 故这一份正文只认 `want` 一个数——它不替任何一族记住"该有几格"。
//!
//! # 为什么只一问（`list`）＋有界重试，不逐个问名
//! 那几位名字的读数归**落格那一侧**（`mount_grants` 每落一位抬一行
//! `system: grant mounted at …`）。本层再 `list` ＋ N 次 `name` 是 N+1 趟往返，而每一趟都
//! 可能**等在门外**（`client.rs::call` 那一推是单槽孔：对面没取走就永远等）⇒ 越少问越不容
//! 易挂在那儿。那一格是**逐位**落上去的（目录先立、各位一位一位落），故"数不满"那一刻是
//! **预期之内**的：到点返回，由调用方那句 `assert_eq!` 落地。

use core::time::Duration;

use env::Wait;

use protocol::service::operator::client::Pane;

/// 数 `pane` 底下到齐没有：一问 `list` ＋ 有界重试（每轮睡 `tick` 毫秒，最多 `budget` 毫秒）。
///
/// 返**这一轮看到的枚数**（数满 `want` 就早返）。返得比 `want` 小 = 期限内没到齐——
/// 那一刻是**可红的**：调用方拿它做 `assert_eq!`，不必在这里 panic（"哪一族差几枚"由
/// 调用方那一行读数说）。
pub fn count_under(pane: &Pane<'_>, want: usize, budget: usize, tick: usize) -> usize {
    let mut left = budget;
    let mut seen = 0usize;
    loop {
        // 一问的期限给 `tick`（**不是**整份额度）：对面这一趟没答就回去睡一拍再来，
        // 额度只在下面那一格上扣——"等了多久"与"问了几趟"因此是同一个数。
        if let Ok(listing) = pane.list(Wait::AtMost(tick)) {
            seen = listing.iter().count();
            if seen >= want {
                return seen;
            }
        }
        if left == 0 {
            return seen;
        }
        let _ = runtime::env::room::sleep(Duration::from_millis(tick as u64));
        left = left.saturating_sub(tick);
    }
}
