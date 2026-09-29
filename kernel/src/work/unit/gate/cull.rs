use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{PieToken, TaskId};

use crate::work::mail::hole::{self, HoleMeta};
use crate::work::mail::nole::{self, NoleMeta};
use crate::work::mail::pole::{self, PoleMeta};
use crate::work::mail::tole::{self, ToleMeta};
use crate::work::unit::task::Task;

use super::pie::AnyPie;
use super::snap::{self, Snap};

fn take(t: &Task, token: PieToken) -> Option<AnyPie> {
    let mut pies = t.pies.lock();
    let pos = pies.iter().position(|p| p.token() == token)?;
    Some(pies.remove(pos))
}

fn pole_meta(pie: &AnyPie) -> Option<Arc<PoleMeta>> {
    match pie {
        AnyPie::Pole(p) => Some(p.meta().clone()),
        AnyPie::Hole(_) | AnyPie::Nole(_) | AnyPie::Tole(_) => None,
    }
}

pub(crate) fn cull(root: (Arc<Task>, PieToken), snap: &Snap) -> usize {
    let (root_task, root_token) = root;
    let mut removed = 0usize;
    let mut unmaps: Vec<(Arc<PoleMeta>, PieToken)> = Vec::new();

    if unmaps.try_reserve(snap.len()).is_err() {
        return 0;
    }
    let mut frontier: Vec<PieToken> = Vec::new();
    if frontier.try_reserve(1).is_err() {
        return 0;
    }
    frontier.push(root_token);

    if let Some(pie) = take(&root_task, root_token) {
        removed += 1;
        let meta = pole_meta(&pie);
        drop(pie);
        if let Some(m) = meta {
            unmaps.push((m, root_token));
        }
    }

    while !frontier.is_empty() {
        let mut next: Vec<PieToken> = Vec::new();
        let mut scan: Vec<(Arc<Task>, PieToken)> = Vec::new();
        for f in frontier.drain(..) {
            let Some(kin) = snap::heirs(f, snap) else {
                break;
            };
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

    for (meta, token) in unmaps {
        let _ = pole::shut(&meta, token);
    }

    removed
}

pub(crate) fn doom(tid: TaskId) {
    let snap = snap::snap();
    let Some(task) = snap::find(tid, &snap) else {
        return;
    };
    // **封印先做，且不依赖分配**（照实记）：这里原先"快照分不出来就 `return`"——于是内存最紧
    // 的那一趟恰好把"主人走了、资源还活着"漏出去。现在是：**封印**先走（`seal_owned` 那条链上
    // 没有一处无保护的分配），**摘副本**（`cull`，它自己也要分配）尽力而为。
    let tokens: Vec<PieToken> = {
        let pies = task.pies.lock();
        let mut v: Vec<PieToken> = Vec::new();
        if v.try_reserve(pies.len()).is_ok() {
            v.extend(pies.iter().map(|p| p.token()));
        }
        v
    };
    seal_owned(tid, &task);
    for token in tokens {
        cull((task.clone(), token), &snap);
    }
}

/// 退场者铸的那些资源，**就地封印**（四族一起）。返封了几枚。
///
/// **不分配，也不在表锁里取别的锁**（照实记：这里原先先收一份 token 快照，`try_reserve` 一失败
/// 就整趟跳过——而"内存紧"与"主人走了、资源还活着"恰好是同一刻）。改成逐枚扫：每轮在表锁里
/// 只做"读主人那一格 ＋ 克隆一枚 `Arc`"（两样都不碰别的锁），**锁外**封印，游标前进一格。
///
/// 两条凭据：`seal` 只置状态、不摘表项 ⇒ 表在整趟里是稳的，游标因此安全；"归它的枚数"在开扫
/// 之前就定了、每轮封掉一枚 ⇒ 循环必收敛。**代价照实记**：每轮重扫到第 `seen` 枚，O(n²)，而
/// `n` 是一个 task 的表长（几十枚量级）——换来的是这条路上一次分配都不需要。
fn seal_owned(tid: TaskId, task: &Arc<Task>) -> usize {
    if tid.get() == 0 {
        return 0;
    }
    let mut sealed = 0;
    let mut seen = 0usize;
    loop {
        let hit = {
            let pies = task.pies.lock();
            pies.iter()
                .filter(|p| p.owner_task() == tid)
                .nth(seen)
                .map(|p| match p {
                    AnyPie::Hole(h) => Resource::Hole(h.meta().clone()),
                    AnyPie::Pole(pl) => Resource::Pole(pl.meta().clone()),
                    AnyPie::Nole(n) => Resource::Nole(n.meta().clone()),
                    AnyPie::Tole(t) => Resource::Tole(t.meta().clone()),
                })
        };
        let Some(res) = hit else {
            break;
        };
        match res {
            Resource::Hole(m) => hole::seal(&m),
            Resource::Pole(m) => pole::seal(&m),
            Resource::Nole(m) => nole::seal(&m),
            Resource::Tole(m) => tole::seal(&m),
        }
        sealed += 1;
        seen += 1;
    }
    sealed
}

enum Resource {
    Hole(Arc<HoleMeta>),
    Pole(Arc<PoleMeta>),
    Nole(Arc<NoleMeta>),
    Tole(Arc<ToleMeta>),
}
