//! 探针共用的量具：**数一块窗格底下到齐没有**。
//!
//! 各族"该有几枚"由**各自的** `Grant::ALL.len()` 说（一枚 `Grant` = 一枚门牌 = 一格），
//! 故这一份正文只认 `want` 一个数——它不替任何一族记住"该有几格"。
//!
//! # 为什么只一问（`list`）＋等事件，不逐个问名
//! 那几位名字的读数归**落格那一侧**（`mount_grants` 每落一位抬一行
//! `system: grant mounted at …`）。本层再 `list` ＋ N 次 `name` 是 N+1 趟往返，而每一趟都
//! 可能**等在门外**（`client.rs::call` 那一推是单槽孔：对面没取走就永远等）⇒ 越少问越不容
//! 易挂在那儿。那一格是**逐位**落上去的（目录先立、各位一位一位落），故"数不满"那一刻是
//! **预期之内**的：到点返回，由调用方那句 `assert_eq!` 落地。
//!
//! # 为什么等的是**事件**，不是"睡一拍再看"
//! 旧版那一拍是 `tick`（20 ms 一睡、睡满 `budget`）——那个窗口与停机扳机赛跑，输了**绿也没有、
//! 红也没有**（`probe-operator-gate` 头注量过：喂了输入的 39 跑里丢 20 次）。改成等事件之后，
//! 等的是"**树真变了**"这一下：没有节拍那一格，"等多久"就是 `budget` 本身。
//!
//! **序是契约**：`Watch::of` 返回即序点——订阅**之前**落的格不在通知义务内，故调用方必须先订、
//! 再把 `watch` 递进来；本函数第一件事仍是**先数一次**（已经到齐的族立刻返回，不必等事件）。

use env::Wait;

use protocol::service::operator::client::{Pane, Watch};

/// **一问的期限**（毫秒）：`list` 那一趟的额度。它**不是节拍**——等事实那一段等的是事件。
const LOOK_MS: usize = 1_000;

/// 数 `pane` 底下到齐没有：一问 `list` ＋ 有界等事件（`watch` 订的是这一块/这一族那条路）。
///
/// 返**数满或到点时看到的枚数**（数满 `want` 就早返）。返得比 `want` 小 = 期限内没到齐——
/// 那一刻是**可红的**：调用方拿它做 `assert_eq!`，不必在这里 panic（"哪一族差几枚"由
/// 调用方那一行读数说）。
pub fn count_under(pane: &Pane<'_>, want: usize, watch: &mut Watch<'_>, budget: usize) -> usize {
    let mut seen = 0usize;
    // 轮数有界：树上多一格才发一条事件 ⇒ 到齐最多 `want` 轮（首问那一趟不算在内）。
    for _ in 0..=want {
        if let Ok(listing) = pane.list(Wait::AtMost(LOOK_MS)) {
            seen = listing.iter().count();
            if seen >= want {
                return seen;
            }
        }
        // **等一条事件**：树真变了才再数一遍；期限内没有事件 = 到点，把最后那一次的枚数交出去。
        if watch.next(Wait::AtMost(budget)).is_err() {
            break;
        }
    }
    seen
}
