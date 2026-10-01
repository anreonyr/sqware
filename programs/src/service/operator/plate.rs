//! operator::plate — **适配**：提示之路上"立一条路"那一句 → 核（`part` / `land`）。
//!
//! 装配者递来的是一整条路（前缀 ＋ 末段）＋ 末段那一枚：`leaf = `[`PieToken::NONE`] 说的是
//! **末段是窗格**。走法只有一句——**前缀逐段 `part` 出来**（`part` 幂等：缺的就地造，已在
//! 就是成了），**末段**按 `leaf` 落叶子或立窗格。
//!
//! **不过门禁**：门禁判的是"**客人**许不许动这一格"，而落一格是本域的本职（`part` / `list` /
//! `seek` / `name` 那四条本来也不判）——本域是这一格的权威。**「改」那一轴同理不问**
//! （[`super::answer`] 的客人那条路才问）：这一条路立的是装配者要的骨架。
//!
//! **挂上去的是装配者交来的那一枚**（帧里带的是它在**本域表里**的号）⇒ 此后客人 `find` 得回
//! 它，而 `Face::of` 认出的"对端"仍是**铸那一枚的那一位**（装配者：它是那一面的服务端）。

use alloc::string::ToString;

use env::PieToken;
use protocol::debug;
use protocol::service::operator::path::Path;
use protocol::service::operator::{Permit, Rule, Where};
use protocol::service::principal::PrincipalId;

use crate::service::operator::core::Operator;

/// **走前缀**：从根起逐段把窗格立出来（`part` 幂等），返**末段该落在的那一块**。
///
/// **照实记（这一手从前还收一本账、还逐段销一次账）**：「归谁改」现在住在那一枚砖上，
/// `part` 把砖顶成窗格时**随砖自然没**——没有第二本账要对齐。
fn walk(tree: &mut Operator, road: &Path) -> Option<Where> {
    let mut at = Where::Root;
    // **前缀那几段**：`parent()` 就是"去掉末段"那一句（std 同形），空路（根）走不到这里
    // ——末段一定有（`plate` 先取了 `file_name()`）。
    let prefix = road.parent()?;
    for seg in prefix.iter() {
        match tree.part(at, seg.to_string()) {
            Ok(id) => at = Where::At(id),
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
pub(super) fn plate(tree: &mut Operator, road: &Path, leaf: PieToken, rule: Rule) {
    let Some(last) = road.file_name() else {
        return debug!("operator: plate empty road");
    };
    let Some(at) = walk(tree, road) else { return };
    // **末段是窗格**（`leaf` 那一格说"这一帧不落叶子"）：立出来就完事——目录不是叶子
    // （没有入口、没有 Pie），故它一处都不落。
    if leaf == PieToken::NONE {
        return match tree.part(at, last.to_string()) {
            Ok(_) => debug!("operator: plate pane {}", last),
            Err(fail) => debug!("operator: plate pane {:?}", fail),
        };
    }
    // **末段是叶子**：没有许可（`Permit::Unset`）＋ **不留主人**（`None`）——与
    // `/svc/sys/principal/{ask,set}` 与 `/svc/sys/coalition/{ask,set}` 那四处门牌同一格：任何已绑身份
    // 都取得回，而"改这一格"不归谁。
    match tree.land(at, last.to_string(), leaf, Permit::Unset, None) {
        Ok(id) => {
            // **带规矩那一轴**（[`Rule::Root`]）：那句规矩是"**许给根**"（`Trunk(ROOT)`）——
            // 它不带号（根那一枚号在这一族的正文里是一枚常量）⇒ 这里**再落一次**把规矩补上
            // （换绑不动号，故号仍是刚铸出来的那个）。**不成只报一行、不中止**：那一格退回
            // "没记许可"，即这一刀之前的行为。
            if let Rule::Root = rule {
                match tree.land(
                    at,
                    last.to_string(),
                    leaf,
                    Permit::Trunk(PrincipalId::ROOT),
                    None,
                ) {
                    Ok(_) => debug!("operator: plate rule root {} id={}", last, id.get()),
                    Err(fail) => debug!("operator: plate rule failed {fail:?}"),
                }
            }
            debug!("operator: plate landed {} id={}", last, id.get())
        }
        Err(fail) => debug!("operator: plate land {:?}", fail),
    }
}
