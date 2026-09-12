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
        // Hole 无映射可撤；Nole 连载荷都没有——两者都不需要"先取强引用"这一步。
        AnyPie::Hole(_) | AnyPie::Nole(_) => None,
    }
}

/// 级联撤销：摘掉 `root`（`task` 表里 `token` 那枚）及其全部后代，并撤销各自
/// 已建立的 Pole 映射。
///
/// 前置：调用方不持任何 L3 锁。
/// 返回：摘掉的门闩数（含 root 本身；root 不在表里则为后代数）。
///
/// # 容量不够就**整个不做**，而不是做一半
///
/// 本条在**退场钩子**里跑（`Hook = fn(usize)`，无错误通道），而它要的每一张表
/// （`unmaps` / `frontier` / `next` / [`snap::heirs`] 的返回）都靠堆。就地逐个
/// `try_reserve` 会把"摘一半"变成可达状态：根已摘、后代的 `unmaps` 没记全 ⇒
/// **Pole 映射泄漏**（映射还在、门闩没了），而那比"这次不级联"糟得多。
///
/// 故**入口处一次备足**，备不出来直接返回 0（一枚未摘，状态原封不动）：
/// 失败是原子的，代价只是这一次放弃级联——内存缓过来后下次退场仍会级联。
pub(crate) fn cull(root: (Arc<Task>, usize), snap: &Snap) -> usize {
    let (root_task, root_token) = root;
    let mut removed = 0usize;
    let mut unmaps: Vec<(Arc<PoleMeta>, usize)> = Vec::new();

    // 上限估计：门闩总数 = 各任务 `pies` 长度之和。取不到容量 ⇒ 放弃本次。
    // （`snap` 是只读快照，长度在本次调用内不变，故这个上界是自洽的。）
    if unmaps.try_reserve(snap.len()).is_err() {
        return 0;
    }
    let mut frontier: Vec<usize> = Vec::new();
    if frontier.try_reserve(1).is_err() {
        return 0;
    }
    frontier.push(root_token);

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
    while !frontier.is_empty() {
        let mut next: Vec<usize> = Vec::new();
        let mut scan: Vec<(Arc<Task>, usize)> = Vec::new();
        for f in frontier.drain(..) {
            // 查询面容量不够 ⇒ 本层到此为止（已摘的照常记着，稍后统一撤映射）。
            let Some(kin) = snap::heirs(f, snap) else { break };
            if scan.try_reserve(kin.len()).is_err() {
                break;
            }
            scan.extend(kin);
        }
        for (t, token) in scan {
            if let Some(pie) = take(&t, token) {
                removed += 1;
                if next.try_reserve(1).is_err() {
                    break;
                }
                next.push(token);
                let meta = pole_meta(&pie);
                drop(pie);
                if let Some(m) = meta {
                    unmaps.push((m, token));
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
/// 签名受 `Hook = fn(usize)` 约束，故自取快照并在其中按 id 找到该任务
/// （此刻收尾者 `reap` 仍持强引用，Weak 可升级）——**不查调度器**。
pub(crate) fn doom(tid: usize) {
    let snap = snap::snap();
    let Some(task) = snap::find(tid, &snap) else {
        return;
    };
    // 先摘出 token 清单（放锁），再逐个 cull——cull 内部会再取同一张表的锁。
    //
    // **清单可失败**：本条是 `Hook = fn(usize)`（**每个任务退场都跑一次**，签名
    // 里没有错误通道），故"这里不可失败"就等于"一个任务退场时堆一紧 ⇒ 整机
    // halt"。容量备不出来就**放弃级联**——与本条开头"找不到该任务就 `return`"
    // 是同一条语义（不做，而不是崩）。
    let tokens: Vec<usize> = {
        let pies = task.pies.lock();
        let mut v: Vec<usize> = Vec::new();
        if v.try_reserve(pies.len()).is_err() {
            return;
        }
        v.extend(pies.iter().map(|p| p.token()));
        v
    };
    for token in tokens {
        cull((task.clone(), token), &snap);
    }
}
