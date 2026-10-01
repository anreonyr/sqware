//! 提示之路上"立一条路"那一句 → 核（part / land）。
//! 装配者递来的是一整条路（前缀 ＋ 末段）＋ 末段那一枚：`leaf = `PieToken::NONE 说的是
//! **末段是窗格**。走法只有一句——**前缀逐段 `part` 出来**（`part` 幂等：缺的就地造，已在
//! 就是成了），**末段**按 `leaf` 落叶子或立窗格。

use alloc::string::ToString;

use env::PieToken;
use protocol::common::path::Path;
use protocol::debug;
use protocol::service::operator::{Permit, Rule, Where};
use protocol::service::principal::PrincipalId;

use crate::service::operator::core::Operator;

/// **走前缀**：从根起逐段把窗格立出来（`part` 幂等），返**末段该落在的那一块**
fn walk(tree: &mut Operator, road: &Path) -> Option<Where> {
    let mut at = Where::Root;
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

/// 失败（路空 / 某一层立不出来 / `land` 拒了）**各报一行读数**：静默退回去会变成"那一格查不到"
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
    // **末段是叶子**：没有许可（Permit::Unset）＋ **不留主人**（`None`）——与
    // `/svc/sys/principal/{ask,set}` 与 `/svc/sys/coalition/{ask,set}` 那四处门牌同一格：任何已绑身份
    match tree.land(at, last.to_string(), leaf, Permit::Unset, None) {
        Ok(change) => {
            let id = change.id;
            // **带规矩那一轴**（Rule::Root）：那句规矩是"**许给根**"（`Trunk(ROOT)`）——
            // （换绑不动号，故号仍是刚铸出来的那个）。**不成只报一行、不中止**：那一格退回
            // 无许可那一档（这一手是"补一句规矩"，`land` 已经成了）。
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
