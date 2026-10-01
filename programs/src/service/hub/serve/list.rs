use super::*;

/// **列册**：这一类此刻有哪几台、哪几台有主（越界答空窗——是答案，不是错误）
pub(super) fn list(ledger: &Ledger, class: String, from: u32, back: PieToken) {
    let window = if ledger.coalition_of(class.clone()).is_some() {
        ledger.list(class, from)
    } else {
        Window {
            status: hub::UNKNOWN,
            ..Window::EMPTY
        }
    };
    let mut tx = Sender::<Window>::from_token(back);
    let _ = tx.send(window);
}
