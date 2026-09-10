// cull — 级联撤销：摘掉一枚门闩及其全部后代。
//
// 与结构面的 `messenger::cull` 同构同名：那边沿 `heir`（强引用）扑杀子域，
// 这边沿 `sire` 反查（查询面）撤销子树。三处共用：`revoke` / `release` / `doom`。
//
// 闭包靠 `snap::heirs` 逐层反查——**不存 heir 列表**（一条关系只存一次）。
//
// 锁纪律：调用方不得持任何 L3。本模块逐任务取放 `Task.pies`（绝不嵌套）；
// 摘除在锁内、**门闩在锁外 drop**（最后一份会跑 `Meta::drop`），Pole 撤映射在
// 全部摘完之后、同样无锁。

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::work::mail::PoleMeta;
use crate::work::mail::pole;
use crate::work::unit::task::Task;

use super::pie::AnyPie;
use super::snap::{self, Snap};

/// 摘掉 `t` 表里 `token` 那枚，返回摘下的门闩（表里没有 → None）。
///
/// **调用方须在锁外 drop 返回值**：它可能是资源实体的最后一份强引用。
fn take(t: &Task, token: usize) -> Option<AnyPie> {
    let mut pies = t.pies.lock();
    let pos = pies.iter().position(|p| p.token() == token)?;
    Some(pies.remove(pos))
}

/// Pole 门闩的资源实体（Hole → None）。**先取强引用、后 drop 门闩**：即便这是
/// 最后一份，unmap 时 Meta 仍活。
fn pole_meta(pie: &AnyPie) -> Option<Arc<PoleMeta>> {
    match pie {
        AnyPie::Pole(p) => Some(p.meta().clone()),
        AnyPie::Hole(_) => None,
    }
}

/// 级联撤销：摘掉 `root`（`task` 表里 `token` 那枚）及其全部后代，并撤销各自
/// 已建立的 Pole 映射。
///
/// 前置：调用方不持任何 L3 锁。
/// 返回：摘掉的门闩数（含 root 本身；root 不在表里则为后代数）。
pub(crate) fn cull(root: (Arc<Task>, usize), snap: &Snap) -> usize {
    let (root_task, root_token) = root;
    let mut removed = 0usize;
    let mut unmaps: Vec<(Arc<PoleMeta>, usize)> = Vec::new();

    // 1. 摘根。
    if let Some(pie) = take(&root_task, root_token) {
        removed += 1;
        let meta = pole_meta(&pie);
        drop(pie); // 锁外：最后一份会跑 Meta::drop
        if let Some(m) = meta {
            unmaps.push((m, root_token));
        }
    }

    // 2. BFS：逐层反查 `sire ∈ frontier` 的子门闩。
    let mut frontier = alloc::vec![root_token];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for f in frontier {
            for (t, token) in snap::heirs(f, snap) {
                if let Some(pie) = take(&t, token) {
                    removed += 1;
                    next.push(token);
                    let meta = pole_meta(&pie);
                    drop(pie);
                    if let Some(m) = meta {
                        unmaps.push((m, token));
                    }
                }
            }
        }
        frontier = next;
    }

    // 3. 无锁段：逐条撤 Pole 映射（幂等；资源已回收则无事）。
    for (meta, token) in unmaps {
        let _ = pole::shut(&meta, token);
    }

    removed
}

/// 退出钩子：任务消亡时，它名下每一枚门闩各自 `cull`（派生链随其断）。
///
/// 签名受 `ExitHook = fn(usize)` 约束，故自取快照并在其中按 id 找到该任务
/// （此刻它仍在 REAPED 里，Weak 可升级）——**不查调度器**。
pub(crate) fn doom(tid: usize) {
    let snap = snap::snap();
    let Some(task) = snap::find(tid, &snap) else {
        return;
    };
    // 先摘出 token 清单（放锁），再逐个 cull——cull 内部会再取同一张表的锁。
    let tokens: Vec<usize> = {
        let pies = task.pies.lock();
        pies.iter().map(|p| p.token()).collect()
    };
    for token in tokens {
        cull((task.clone(), token), &snap);
    }
}
