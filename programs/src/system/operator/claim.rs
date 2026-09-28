//! operator::claim — **载体**：本域那张表上按「谁开的 ＋ 记号」认领那几枚孔。
//!
//! 树这一侧要用到的孔各有各的记号（答话路 / 问话孔 / 门牌），而**认领的判据只有一条**：
//! `owner == who` 且 `mark == 记号`。这一份是那条判据的唯一正文——[`super::server`]（收那三句话）
//! 与 [`super::door`]（门外那一问要的两枚门牌）都从这里取。**它不认识树、也不认识门**：
//! 记号那一侧是调用方给的（`protocol::system::operator` 的 `LINK` / `ASK_MARK` / 七位各一枚，
//! 以及板的 `ENTRY_MARK`）。

use env::{Mark, PieToken, TaskId};
use protocol::communication::establish;
use protocol::debug;
use protocol::system::board as bcall;
use protocol::system::operator::LINK;
use runtime::env::mail;

/// 这一枚孔刻的是哪一枚记号（**本域表里那一枚的第三格**）。
///
/// 读不出（不在本表里 / 不是孔 / 已封印）⇒ [`Mark::NONE`]——它不是任何一面，故
/// [`grant_of`](protocol::system::operator::grant::grant_of) 答 `None`、[`ask_of`] 也认不回它
/// （两处同一句）。
pub(super) fn mark_of(ask: PieToken) -> Mark {
    establish::marked_as(ask).unwrap_or(Mark::NONE)
}

/// **认领恰好一枚**：按「谁开的 + 记号」扫全表，答**第一枚**。
///
/// 三处认领（答话路 / 问话孔 / 门牌）原先各写一遍同一段扫表，且**都取第一枚而从不看有几枚**。
/// 这一格把那段扫表合成一处；`more` 是"多枚要不要说一句"。
///
/// 这个过滤器**分不出副本与"第二扇门"，也不该分**。故：
///
/// | 处 | 记号 | 多枚是 |
/// |---|---|---|
/// | [`reply_of`] | `LINK` | **不可能**——装配者那一条路只 `endpoint` 一次（"同一位、同一记号只可能有一枚"从闸变成了构造） |
/// | [`find_face`] | `ENTRY` | **结构性正常**（同一域交多枚：`land` 交一枚、门禁交一枚）⇒ 不说 |
/// | [`ask_of`] | `ASK` | **契约被破**：一个域只该铸一枚问话孔（裸 `unseal_hole`，没有同名闸），多出来的那枚永远没人读它的推 ⇒ 说一句 |
///
/// 而"取第一枚"在三处都正当：命中的几枚背后是**同一扇门**（同一份 `HoleMeta`），任一枚都通。
///
/// 还有两格记着（不在这一刀里）：
///
/// - **别把它做成 fail-closed**：两枚孔的出现与持树者查表之间有**天然竞态**（持树者每 1ms 查
///   一次，而两枚孔之间只隔两个 envcalls）⇒ "拒"是间歇的，且那位客人从此没人给它挂孔
///   （持树者会永远停在"还有人没挂上"那一档）；
/// - **干净的关法**是让问话孔与入口那一枚也走**一次 `establish::endpoint`**（那一手与
///   "只铸一枚"同形），把"只可能有一枚"从纪律变成**构造**——那是客侧形状的改动，另一刀。
pub(super) fn claim(mark: Mark, who: TaskId, more: Option<&str>) -> Option<PieToken> {
    let mut hits = mail::pies().filter(|p| p.owner == who && p.mark == mark);
    let first = hits.next()?;
    // **第二枚 ⇒ "只可能有一枚"那条纪律破了**：说话（`more` 那一格就是这句话）。
    if hits.next().is_some() {
        if let Some(note) = more {
            debug!("{}", note);
        }
    }
    Some(first.token)
}

/// 转授来的那一枚答话路（**写端**，落在本表里）。
///
/// 两格判据，都是确定的号：`owner == who`（那扇门是**这位客人**开的，副本共享同一事实）＋
/// 记号 == `operator`（那一枚是**树路**上的一枚）。
pub(super) fn reply_of(who: TaskId) -> Option<PieToken> {
    // 多枚不可能（**装配者那一条路只 `endpoint` 一次**）⇒ 不说。
    claim(Mark::of(LINK), who, None)
}

/// 这一位客人**自己**交来的那一枚问话孔。
///
/// 判据两格，缺一不可：`owner == who`（那扇门是它开的）**且** 记号 == `mark`（它亲手铸的
/// 那一枚）——客人交来的**入口**也满足前两格（都是它铸、它交的），两件事只有记号分得开。
///
/// **`mark` 那一格由调用方逐枚给**（[`super::server`] 的 `MARKS`）：控制面那一枚 ＋ 七位操作面
/// 各一枚——客人开在哪一位上，只有它那枚孔的记号说得清。
pub(super) fn ask_of(who: TaskId, mark: Mark) -> Option<PieToken> {
    // 多枚**是契约被破**（一位客人只该在一位上铸一枚问话孔）⇒ 说一句。
    claim(mark, who, Some("operator: two asks"))
}

/// 找**某一位域**交给本域的那枚服务门牌（`opened_by == who` 且记号是服务入口）。
///
/// 门牌那一枚走的是**裸 `unseal_hole`**，故"一个域只交一枚"同样是纪律而不是判据。
pub(super) fn find_face(who: TaskId) -> Option<PieToken> {
    // 多枚**正常**（副本共享 `opened_by`：`land` 交一枚、门禁交一枚）⇒ 不说。
    claim(bcall::ENTRY_MARK, who, None)
}
