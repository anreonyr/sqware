//! hub::serve::list — **列册那一面**：问这一格有哪些设备（按类列，答一窗）。

use super::*;

/// **列册**：这一类此刻有哪几台、哪几台有主（越界答空窗——是答案，不是错误）。
pub(super) fn list(ledger: &Ledger, class: String, from: u32, back: PieToken) {
    let window = if ledger.coalition_of(class.clone()).is_some() {
        ledger.list(class, from)
    } else {
        Window {
            status: hub::UNKNOWN,
            ..Window::EMPTY
        }
    };
    // **写端跟着这一趟走**：落出作用域时等这只手被取走（`Drop`）——那一位客人不来取，卡的是
    // 他自己那一趟。**一枚孔一枚写端**，共用一格那种错编不出来。
    let mut tx = Sender::<Window>::from_token(back);
    let _ = tx.send(window);
}
