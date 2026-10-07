//! 树这一侧要用到的孔各有各的记号（答话路 / 问话孔 / 门牌），而认领的判据只有一条：
//! `owner == who` 且 `mark == 记号`。这一份是那条判据的唯一正文——super::server（收那三句话）
//! 记号那一侧是调用方给的（system_client::operator 的 `LINK` / `ASK_MARK` / 每一位各一枚，
//! 以及各族交过来的门牌：名册那一族**两面各一枚**、盟册那一族仍是通用那枚 `entry`）。

use env::{Mark, PieToken, TaskId};
use ipc::session::establish;
use programs::debug;
use system_api::operator::LINK;
use ::resource::raw::{pies};

/// 读不出（不在本表里 / 不是孔 / 已封印）⇒ Mark::NONE——它不是任何一面，故
/// grant_of 答 `None`、ask_of 也认不回它
/// （两处同一句）
pub(super) fn mark_of(ask: PieToken) -> Mark {
    establish::marked_as(ask).unwrap_or(Mark::NONE)
}

/// **认领恰好一枚**：按「谁开的 + 记号」扫全表，答**第一枚**
pub(super) fn claim(mark: Mark, who: TaskId, more: Option<&str>) -> Option<PieToken> {
    let mut hits = pies().filter(|p| p.owner == who && p.mark == mark);
    let first = hits.next()?;
    // **第二枚 ⇒ "只可能有一枚"那条纪律破了**：说话（`more` 那一格就是这句话）。
    if hits.next().is_some() {
        if let Some(note) = more {
            debug::put(note);
        }
    }
    Some(first.token)
}

/// 转授来的那一枚答话路（**写端**，落在本表里）
/// 两格判据，都是确定的号：`owner == who`（那扇门是**这位客人**开的，副本共享同一事实）＋
/// 记号 == `operator`（那一枚是**树路**上的一枚）
pub(super) fn reply_of(who: TaskId) -> Option<PieToken> {
    claim(Mark::of(LINK), who, None)
}

/// 这一位客人**自己**交来的那一枚问话孔
/// 判据两格，缺一不可：`owner == who`（那扇门是它开的）**且** 记号 == `mark`（它亲手铸的
/// 那一枚）——客人交来的**入口**也满足前两格（都是它铸、它交的），两件事只有记号分得开
/// **`mark` 那一格由调用方逐枚给**（super::server 的 `MARKS`）：控制面那一枚 ＋ 操作面每一位
/// 各一枚——客人开在哪一位上，只有它那枚孔的记号说得清
pub(super) fn ask_of(who: TaskId, mark: Mark) -> Option<PieToken> {
    // 多枚**是契约被破**（一位客人只该在一位上铸一枚问话孔）⇒ 说一句。
    claim(mark, who, Some("operator: two asks"))
}
