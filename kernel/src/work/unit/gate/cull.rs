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

/// 退场那一趟：**先封印，再看快照摘副本**。
///
/// **封印不看快照**（照实记）：退场钩子递过来的就是这一具 `Task`，`seal_owned` 只需要它
/// 与自己那张 `pies` 表。从前这里先 `snap::snap()` ＋ `snap::find(tid)` 才动手——而
/// `roster()` **自己要分配**，备不出容量就返回空表（见 `scheduler::core::table::roster`）
/// ⇒ 那一趟一枚都不封，"主人走了、资源还活着"恰好落在内存最紧的一刻。这条缝现在堵死：
/// 快照只用来**摘副本**（`cull` 要沿 `sire` 边找跨表的接续，它自己也要分配 ⇒ 尽力而为）。
pub(crate) fn doom(task: &Arc<Task>) {
    let tid = task.ident.id;
    let _ = seal_owned(tid, task);
    // **摘副本尽力而为**：它要分配（token 快照、frontier、unmaps），备不出就只少摘几枚
    // 副本——副本被摘是清账，不是判死（判死已经在上面做完了）。
    let tokens: Vec<PieToken> = {
        let pies = task.pies.lock();
        let mut v: Vec<PieToken> = Vec::new();
        if v.try_reserve(pies.len()).is_ok() {
            v.extend(pies.iter().map(|p| p.token()));
        }
        v
    };
    if tokens.is_empty() {
        return;
    }
    let snap = snap::snap();
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
