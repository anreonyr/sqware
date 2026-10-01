//! system::common::face::carrier — **一只组守着 N 枚门牌**：从哪一枚收到就是哪一面。
//! ```text
//!   carrier(那几枚孔，各自代表哪一面) → 组里那几枚就绪 ⇒ 收到哪一枚孔 = 哪一面
//!   on(面, 发送者, 帧)                每一句话交给谁（那一族自己的门）
//! ```
//! **它只有一条判断**："这一帧从哪一枚孔进来"。名册与盟册**都没有会话**（门牌自己就是那条路，
//! 所有人往同一枚孔推帧），故"面"没有别处可读——它就是这一枚孔。

use alloc::vec::Vec;

use env::Wait;
use env::{HoleDir, PieToken, TaskId};
use runtime::PAGE_SIZE;
use runtime::core::res::pile::Pile;
use runtime::env::mail::HolePie;

use crate::system::common::life::service::Start;
use crate::unit::Died;

/// **守着这几枚门牌**，直到组坏掉：一场一句话地交给 `on`。
/// `faces` = （那一枚孔，它代表哪一面）——**面只有这一条来路**；`on` = 一条帧怎么办
/// （哪一面进来的、内核盖的发送者、帧本身）。
/// 返 `Err`：起手那两样备不下（`Desk`：那只组；`Room`：收帧那一页）或**组坏了**（`Dead`）。
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
    // 二、收帧那一页：**备一次**，循环里一直用（一页 = 余量：本仓的帧都在几十到几百字节）。
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
        while let Ok((len, from)) = hole.pull(&mut buf, Wait::POLL) {
            on(face, from, &buf[..len]);
        }
    }
}
