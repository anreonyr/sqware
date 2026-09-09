// snap — 全世界任务快照 + 沿 sire 的查询面。
//
// 能力面**只存一条边**（`Pie.sire`）。另两个方向都靠查询还原：
//   - `vestor(pie, snap)` —— 向上：父门闩的持有者（= 授与人）
//   - `heirs(token, snap)` —— 向下：`sire` 指向我的那些子门闩
//   - `vestable(pie, dst, snap)` —— BACK 守门
//   - `find(tid, snap)` —— 按 id 取任务（退出钩子用）
//
// **快照由适配层提供**（boot 注入 provider）：本模块不依赖 scheduler。快照是
// `Weak<Task>` 列表——死条目升级失败即跳过，故查询无副作用、无需清理。
//
// 锁序：本模块逐任务取放 `Task.pies`（L3），**绝不嵌套**——调用方不得持 L3 调用。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use crate::lock::OnceLock;
use crate::work::unit::task::Task;

use super::pie::{AnyPie, Permission};

/// 全世界任务快照：查询与级联的唯一输入。只读、不增删。
pub(crate) type Snap = [Weak<Task>];

/// 快照提供者（boot 注入；`gate` 不依赖 scheduler）。
fn provider() -> &'static OnceLock<fn() -> Vec<Weak<Task>>> {
    static P: OnceLock<fn() -> Vec<Weak<Task>>> = OnceLock::new();
    &P
}

/// 注入快照提供者（boot 一次性调用）。
pub(crate) fn install(f: fn() -> Vec<Weak<Task>>) {
    let _ = provider().set(f);
}

/// 拍一张快照。未注入 → 空（查询退化为「找不到」，即不级联、不认亲）。
pub(crate) fn snap() -> Vec<Weak<Task>> {
    provider().get().copied().map(|f| f()).unwrap_or_default()
}

/// 按 task id 取任务（升级 Weak；不在 → None）。
pub(crate) fn find(tid: usize, snap: &Snap) -> Option<Arc<Task>> {
    for w in snap {
        if let Some(t) = w.upgrade()
            && t.ident.id == tid
        {
            return Some(t);
        }
    }
    None
}

/// 向上：这枚门闩的授与人（= 父门闩所在任务的 id）。原始自持 → None。
pub(crate) fn vestor(pie: &AnyPie, snap: &Snap) -> Option<usize> {
    holder(pie.sire()?, snap)
}

/// 某枚门闩（按 token）的持有者：快照里谁的表里有它。
fn holder(token: usize, snap: &Snap) -> Option<usize> {
    for w in snap {
        let Some(t) = w.upgrade() else { continue };
        // 显式作用域：guard 必须在取 id 之前释放（不跨表）。
        let found = {
            let pies = t.pies.lock();
            pies.iter().any(|p| p.token() == token)
        };
        if found {
            return Some(t.ident.id);
        }
    }
    None
}

/// 向下：`sire == token` 的全部子门闩（持有者 + 子 token）。
pub(crate) fn heirs(token: usize, snap: &Snap) -> Vec<(Arc<Task>, usize)> {
    let mut out = Vec::new();
    for w in snap {
        let Some(t) = w.upgrade() else { continue };
        let kids: Vec<usize> = {
            let pies = t.pies.lock();
            pies.iter()
                .filter(|p| p.sire() == Some(token))
                .map(|p| p.token())
                .collect()
        };
        for k in kids {
            out.push((t.clone(), k));
        }
    }
    out
}

/// BACK 守门：带 BACK 的源只能授给 `sire` 的持有者；不带 BACK 恒真；
/// 原始自持（sire = None）带 BACK 不受限（回授目标自由）。
pub(crate) fn vestable(pie: &AnyPie, dst: usize, snap: &Snap) -> bool {
    if !pie.permission().contains(Permission::BACK) {
        return true;
    }
    match pie.sire() {
        None => true,
        Some(sire) => holder(sire, snap) == Some(dst),
    }
}
