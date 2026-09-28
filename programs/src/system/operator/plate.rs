//! operator::plate — **适配**：提示之路上"立一条路"那一句 → 核（`part` / `land`）＋ 账。
//!
//! 装配者递来的是一整条路（前缀 ＋ 末段）＋ 末段那一枚：`leaf = `[`PieToken::NONE`] 说的是
//! **末段是窗格**。走法只有一句——**前缀逐段 `part` 出来**（`part` 幂等：缺的就地造，已在
//! 就是成了），**末段**按 `leaf` 落叶子或立窗格。
//!
//! **不过门禁**：门禁判的是"**客人**许不许动这一格"，而落一格是本域的本职（`part` / `list` /
//! `seek` / `name` 那四条本来也不判）——本域是这一格的权威。
//!
//! **挂上去的是装配者交来的那一枚**（帧里带的是它在**本域表里**的号）⇒ 此后客人 `find` 得回
//! 它，而 `Face::of` 认出的"对端"仍是**铸那一枚的那一位**（装配者：它是那一面的服务端）。

use env::{Name, PieToken};
use protocol::debug;
use protocol::system::operator::{Permit, Where};

use crate::system::operator::core::Operator;
use crate::system::operator::core::ledger::Book;

/// **走前缀**：从根起逐段把窗格立出来（`part` 幂等），返**末段该落在的那一块**。
///
/// **那个窄口子照销**：`part` 会把一枚 `Tile` **静默顶成**一块 `Pane`——那一格已经不是"放 Pie
/// 的那一格"了，故账上那一行要跟着销（同 [`super::answer`] 的 `Part` 那一支）。`Book::drop` 对
/// "账上没这一行"是无声的（它只是缓存）。
fn walk(tree: &mut Operator, book: &mut Book, road: &[Name]) -> Option<Where> {
    let mut at = Where::Root;
    for seg in &road[..road.len() - 1] {
        match tree.part(at, *seg) {
            Ok(id) => {
                book.drop(id);
                at = Where::At(id);
            }
            Err(fail) => {
                debug!("operator: plate walk {:?}", fail);
                return None;
            }
        }
    }
    Some(at)
}

/// **装配者要本域立的那一条路**：前缀逐段立窗格（缺的就地造），末段按 `leaf` 落叶子或立窗格。
///
/// 失败（路空 / 某一层立不出来 / `land` 拒了）**各报一行读数**：静默退回去会变成"那一格查不到"。
pub(super) fn plate(tree: &mut Operator, book: &mut Book, road: &[Name], leaf: PieToken) {
    let Some(last) = road.last().copied() else {
        return debug!("operator: plate empty road");
    };
    let Some(at) = walk(tree, book, road) else { return };
    // **末段是窗格**（`leaf` 那一格说"这一帧不落叶子"）：立出来就完事——目录不是叶子
    // （没有入口、没有 Pie），故它一处都不落。
    if leaf == PieToken::NONE {
        return match tree.part(at, last) {
            Ok(id) => {
                book.drop(id);
                debug!("operator: plate pane {}", last.as_str())
            }
            Err(fail) => debug!("operator: plate pane {:?}", fail),
        };
    }
    // **末段是叶子**：没有许可（`Permit::Unset`）＋ **不留主人**（`mine = false`）——与
    // `/sys/principal` / `/sys/coalition` 两处门牌同一格：任何已绑身份都取得回，而"改这一格"
    // 不归谁。
    let who = runtime::env::unit::sire();
    match book.land(at, last, leaf, false, who, || {
        tree.land(at, last, leaf, Permit::Unset)
    }) {
        Ok(id) => debug!("operator: plate landed {} id={}", last.as_str(), id.get()),
        Err(fail) => debug!("operator: plate land {:?}", fail),
    }
}
