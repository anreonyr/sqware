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

use protocol::system::operator::{Pane, Watch};

/// **一问的期限**（毫秒）：`list` 那一趟的额度。它**不是节拍**——等事实那一段等的是事件。
const LOOK_MS: usize = 1_000;

/// 数 `pane` 底下到齐没有：一问 `list` ＋ 有界等事件（`watch` 订的是这一块/这一族那条路）。
///
/// 返**数满或到点时看到的枚数**（数满 `want` 就早返）。返得比 `want` 小 = 期限内没到齐——
/// 那一刻是**可红的**：调用方拿它做 `assert_eq!`，不必在这里 panic（"哪一族差几枚"由
/// 调用方那一行读数说）。
///
/// **到点那一趟仍要问树一次**（这一条是量出来的）：本函数量的是"**树上此刻有几格**"，
/// 不是"我收到过几条事件"。两件事在这里会分家，各有一条实测过的路：
/// · `list` 那一问**可以答不上来**（`Wait::AtMost(LOOK_MS)` 是期限，不是保证）——
///   从前那一格不改 `seen`，于是"没答上来"被留在读数里当了"没有"；
/// · 事件那一路**可以漏**（那一具架是共享的 `PANE_CAP` 格：手所指的那一格在环绕回来之后
///   可能已经被别条路顶掉，订阅那一侧按"路对不上"丢）——于是最后那一次 `list` 停在 3。
/// 全控制台量到过这一形：`probe-control` 数到 3/4，而四条 `system: control mounted at` 都印了。
pub fn count_under(pane: &Pane<'_>, want: usize, watch: &mut Watch<'_>, budget: usize) -> usize {
    let mut seen = 0usize;
    // **两条"没看见"分开记**：`blind` = 那一问没答上来（期限到了）；`quiet` = 等事件到点。
    // 它们是两种不同的下一步，故不能合并成"反正没到齐"。
    let mut blind = 0usize;
    let mut quiet = 0usize;
    // 轮数有界：树上多一格才发一条事件 ⇒ 到齐最多 `want` 轮（首问那一趟不算在内）。
    for _ in 0..=want {
        match pane.list(Wait::AtMost(LOOK_MS)) {
            Ok(listing) => {
                seen = listing.iter().count();
                if seen >= want {
                    return seen;
                }
            }
            Err(_) => blind = blind.saturating_add(1),
        }
        // **等一条事件**：树真变了才再数一遍；期限内没有事件 = 到点。
        if watch.next(Wait::AtMost(budget)).is_err() {
            quiet = 1;
            break;
        }
    }
    // **到点：最后再问树一次**（上面两条"没看见"的路都在这里被兜住）。
    if let Ok(listing) = pane.list(Wait::AtMost(LOOK_MS)) {
        seen = listing.iter().count();
    }
    if seen < want {
        // **只在数不满时报**（release 也看得见）：两种成因分得开，下一轮才查得下去。
        protocol::debug::put(&alloc::format!(
            "probe: count short seen={seen} want={want} blind={blind} quiet={quiet}"
        ));
    }
    seen
}
