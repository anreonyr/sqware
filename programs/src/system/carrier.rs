//! system::carrier — **一只组守着 N 枚门牌**：从哪一枚收到就是哪一面。
//!
//! ```text
//!   carrier(那几枚孔，各自代表哪一面) → 组里那几枚就绪 ⇒ 收到哪一枚孔 = 哪一面
//!   on(面, 发送者, 帧)                每一句话交给谁（那一族自己的门）
//! ```
//!
//! **它只有一条判断**："这一帧从哪一枚孔进来"。名册与盟册**都没有会话**（门牌自己就是那条路，
//! 所有人往同一枚孔推帧），故"面"没有别处可读——它就是这一枚孔。
//!
//! **把这一趟收进来的量**（见 [`carrier`] 自己的照实记）：名册与盟册两处的常驻段
//! **各 31 码行、逐字同构**（差的只有族名与那一句注）。
//!
//! **持树者那一处不在这里**：它等的是**两个不同来路**（门 + 提示之路），而且面长在**会话**上
//! （记号那一格）——不是"哪一枚孔"⇒ 形状不同，不并。

use alloc::vec::Vec;

use env::Wait;
use env::{HoleDir, PieToken, TaskId};
use runtime::PAGE_SIZE;
use runtime::core::pile::Pile;
use runtime::env::mail::HolePie;

use super::control::service::Start;
use crate::program::Died;

/// **守着这几枚门牌**，直到组坏掉：一场一句话地交给 `on`。
///
/// `faces` = （那一枚孔，它代表哪一面）——**面只有这一条来路**；`on` = 一条帧怎么办
/// （哪一面进来的、内核盖的发送者、帧本身）。
///
/// 返 `Err`：起手那两样备不下（`Desk`：那只组；`Room`：收帧那一页）或**组坏了**（`Dead`）。
///
/// # 照实记（收进来的这一趟是量出来的：两台逐字同构）
///
/// ```text
///   名册 server 的常驻段   31 码行   /svc/principal/{ask,set}   两枚门牌 → 两处 turn
///   盟册 server 的常驻段   31 码行   /svc/coalition/{ask,set}   同上
/// ```
///
/// 逐行 diff 出来的差异只有三处，都不是结构：族名（`pcall` / `ccall`）、返回的元组里多装一样
/// （名册多一本 `book` 之外的身份面）、以及各自那一句注。**步骤一字不差**：立组 → 挂 N 枚 →
/// 备一页 → 等 → 从哪一枚读到就是哪一面 → 把这一批取干净 → 交给门。
///
/// **照实记（`Ok(None)` 不是终局：名册那一台在这一格栽过）**：它原先把等那一处写成了
/// `let Ok(Some(hit)) = pile.await_(…) else { 死 }`——那是从持树者那一处抄来的形状，而那一处
/// `else` 里是 `continue`。`None`（挂起过、或期限到）与 `Err`（组死了）是两件事，折叠它就是把
/// "这一轮没事"读成"这一组死了"。实测：名册答完**第一帧**就当"组死了"退场，整机装配随之塌
/// （`exit tid=5 note: inner: group dead`）。
///
/// **照实记（那一页为什么在这一层备）**：`buf` 原先各自在 `serve` 的起手闭包里备、随元组交出来
/// ——它只被这一趟用。收进来之后它落在这一层：起手的失败域从"闭包里 `Err(Start::Room)`"变成
/// "这一手答 `Err(Start::Room)`"，**同一格、同一个号**，只是报的时点从起手挪到常驻前一句。
pub fn carrier<G: Copy>(
    died: Died,
    faces: &[(PieToken, G)],
    mut on: impl FnMut(G, TaskId, &[u8]),
) -> Result<(), Start> {
    // 一、那只组：一枚成员一枚孔（"就绪"挂进组，"取消息"仍走各自那一手）。
    let pile = Pile::unseal(false).map_err(|_| Start::Desk(died))?;
    for (entry, _) in faces {
        pile.attach(&HolePie::from_token(*entry), HoleDir::Pull)
            .map_err(|_| Start::Desk(died))?;
    }
    // 二、收帧那一页：**备一次**，循环里一直用（一页 = 载体的界：任何一条消息一趟都取得出来）。
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(PAGE_SIZE).is_err() {
        return Err(Start::Room(died));
    }
    buf.resize(PAGE_SIZE, 0);
    // 三、常驻：这是常态，故等待没有期限。
    loop {
        let (tok, _dir) = match pile.await_(Wait::Forever) {
            Ok(Some(hit)) => hit,
            Ok(None) => continue,
            Err(_) => return Err(Start::Dead(died)),
        };
        // **余下的号**（构造上到不了：组里只挂了这几枚）⇒ 不猜，回去再等。
        let Some(face) = faces
            .iter()
            .find(|(entry, _)| *entry == tok)
            .map(|(_, face)| *face)
        else {
            continue;
        };
        // 门牌是**单槽**：一次醒来的这一批要取干净（可能不止一位客人）。
        let hole = HolePie::from_token(tok);
        while let Ok((len, from)) = hole.pull_timeout_from(&mut buf, Wait::POLL) {
            on(face, from, &buf[..len]);
        }
    }
}
