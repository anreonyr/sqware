//! order — **这一张单自己算不了的那一件事：次序**。
//!
//! **照实记（层六·2 第二块：从 `mod.rs` 切出来）**：这一块（`order_scene` 与它下面那几具判据）
//! 整体搬到同名目录下的 `order.rs`——**两个读者共用这一份**（装配者算起台的次序、`crates/image`
//! 算装载次序），故它是「这一层的算法」那一半。**模块名不变**：`mod.rs` 里 `pub use order::*;`
//! 把这一块原样摆回 `crate::program` 那个名字空间 ⇒ 全仓引用一处都不用动。

use super::{Program, is_target};

// ── 这一张单自己算不了的那一件事：**次序**（两个读者共用这一份）──────────────

/// 图上说不通的那三种——每一种都报出**名字**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepsFail {
    /// 一条边指着本单里没有的台（名字）。
    Unknown(&'static str),
    /// 被指着的那一台**没有凭据**（`demand.supply` 空 ⇒ 它交不出"我答得动"）。
    NoEvidence(&'static str),
    /// 取不出可排的台 ⇒ 环（名字 = 卡住的那一个）。
    Cycle(&'static str),
}

/// **按 `after` 把这一张单排成次序**（拓扑，原地重排）：每条边都在前面；等 [`SCENE`]（这一趟走完）
/// 的排**最后**。
/// 同一批按**名字**排（与声明次序无关，可复现）。图上那三种说不通当场挑出来（[`DepsFail`]）。
///
/// **两个读者共用这一份**（照实记）：宿主那一侧打包时校验（`crates/image`，报得出名字），
/// 引导期那一趟排次序（`system::assemble`）。故它**不许分配**——本文件是宿主安全的
/// （只许引 `env`，`crates/image` 用 `#[path]` 文本包含它），只用切片与定长栈。
///
/// **它不解释任何一台的字段**：只读 [`Relation::after`] 那一格（[`SCENE`] 只是其中一个名字）。
pub fn order_scene(list: &mut [&'static Program]) -> Result<(), DepsFail> {
    // 一、每条边都要落得下：指得到本单里的台，且那一台说得出"我答得动"。
    //     **[`SCENE`] 那一条除外**：它指的是这一趟自己，不是本单里的台，也没有"答得动"可言
    //     （见 [`SCENE`] 的头注）。
    let mut i = 0;
    while i < list.len() {
        if let Some(deps) = list[i].relation.after {
            let mut d = 0;
            while d < deps.len() {
                let name = deps[d];
                // **[目标单元](Kind::Target)那一条除外**：它指的是这一趟自己，不是本单里的台，
                // 也没有"答得动"可言（见 [`SCENE_UNIT`] 与 [`is_target`]）。
                if !is_target(name) {
                    match find(list, name) {
                        None => return Err(DepsFail::Unknown(name)),
                        Some(target) if target.demand.supply.is_empty() => {
                            return Err(DepsFail::NoEvidence(name))
                        }
                        Some(_) => {}
                    }
                }
                d += 1;
            }
        }
        i += 1;
    }
    // 二、拓扑：一轮取"前置都排好了"的那一个；同批挑名字最小的（可复现）。
    let n = list.len();
    let mut placed = 0usize;
    while placed < n {
        let mut pick: Option<usize> = None;
        let mut i = placed;
        while i < n {
            if !waits_scene(list[i]) && ready(list, i, placed) {
                match pick {
                    Some(best) if list[best].name() <= list[i].name() => {}
                    _ => pick = Some(i),
                }
            }
            i += 1;
        }
        let Some(i) = pick else { break };
        list.swap(placed, i);
        placed += 1;
    }
    // 三、收尾：剩下的必须全是等[目标单元](Kind::Target)的（不是 ⇒ 环）；它们同批按名字。
    let mut i = placed;
    while i < n {
        if !waits_scene(list[i]) {
            return Err(DepsFail::Cycle(list[i].name()));
        }
        i += 1;
    }
    let mut i = placed;
    while i < n {
        let mut pick = i;
        let mut j = i + 1;
        while j < n {
            if list[j].name() < list[pick].name() {
                pick = j;
            }
            j += 1;
        }
        list.swap(i, pick);
        i += 1;
    }
    Ok(())
}

/// **这一台等的是"这一趟走完"吗**——`after` 里有一条边指着[目标单元](Kind::Target)就是。
///
/// 它有两个读者，判的是同一句话：[`order_scene`] 据它把这一台排到最后（那一格要到那时才到点），
/// 而装配那一趟据它跳过那一条边（`Assembly::assemble`：等一个"这一趟"没有可等的对象）。
fn waits_scene(program: &Program) -> bool {
    program
        .relation
        .after
        .is_some_and(|deps| deps.iter().any(|name| is_target(name)))
}

/// 这一台的**边都排好了吗**（`list[..placed]` 里找得到每一条边指着的那一台）。
fn ready(list: &[&'static Program], i: usize, placed: usize) -> bool {
    let Some(deps) = list[i].relation.after else {
        return true;
    };
    let mut d = 0;
    while d < deps.len() {
        let mut found = false;
        let mut j = 0;
        while j < placed {
            if list[j].name() == deps[d] {
                found = true;
                break;
            }
            j += 1;
        }
        if !found {
            return false;
        }
        d += 1;
    }
    true
}

/// 本单里按名字找那一台（**只查不比存** ⇒ 借 `&str`）。
fn find<'a>(list: &[&'a Program], name: &str) -> Option<&'a Program> {
    let mut i = 0;
    while i < list.len() {
        if list[i].name() == name {
            return Some(list[i]);
        }
        i += 1;
    }
    None
}

