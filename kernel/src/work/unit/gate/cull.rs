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
    let pie = pies.remove(pos);
    pie.invalidate();
    Some(pie)
}

pub(crate) struct Cleanup {
    root: Option<AnyPie>,
    removed: Vec<AnyPie>,
}

impl Cleanup {
    pub(crate) fn finish(self) -> usize {
        let count = self.removed.len() + usize::from(self.root.is_some());
        for pie in self.root.into_iter().chain(self.removed) {
            if let AnyPie::Pole(p) = &pie {
                pole::shut(p.meta(), p.token).expect("cull: unmap token");
            }
            drop(pie);
        }
        count
    }
}

pub(crate) fn cull(root: (Arc<Task>, PieToken), snap: &Snap) -> Cleanup {
    let (root_task, root_token) = root;
    let sole = root_task.pies.lock().iter().find(|pie| pie.token() == root_token)
        .is_some_and(|pie| matches!(pie, AnyPie::Pole(p) if p.meta().backing().exclusive() && p.heir.is_none()));
    if sole { return Cleanup { root: take(&root_task, root_token), removed: Vec::new() }; }
    let mut root = None;
    let mut removed = Vec::new();
    let mut frontier = Vec::new();
    // Reserve for every actual token before changing the graph.
    let count = snap.iter().filter_map(|weak| weak.upgrade())
        .map(|task| task.pies.lock().len()).sum::<usize>() + 1;
    if removed.try_reserve(count).is_err() || frontier.try_reserve(count).is_err() {
        return Cleanup { root, removed };
    }
    root = take(&root_task, root_token);
    frontier.push(root_token);
    let mut cursor = 0;
    while cursor < frontier.len() {
        let token = frontier[cursor];
        cursor += 1;
        let Some(kin) = snap::heirs(token, snap) else { break };
        for (task, token) in kin {
            if let Some(pie) = take(&task, token) {
                frontier.push(token);
                removed.push(pie);
            }
        }
    }
    Cleanup { root, removed }
}

/// 退场那一趟：**先封印，再看快照摘副本**。
pub(crate) fn doom(task: &Arc<Task>) {
    let graph = super::GRAPH.lock();
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
    let mut cleanups = Vec::new();
    if cleanups.try_reserve(tokens.len()).is_err() { return; }
    for token in tokens { cleanups.push(cull((task.clone(), token), &snap)); }
    drop(graph);
    for cleanup in cleanups { cleanup.finish(); }
}

/// 退场者铸的那些资源，**就地封印**（四族一起）。返封了几枚。
///
/// 两条凭据：`seal` 只置状态、不摘表项 ⇒ 表在整趟里是稳的，游标因此安全；"归它的枚数"在开扫
/// 之前就定了、每轮封掉一枚 ⇒ 循环必收敛。**代价**：每轮重扫到第 `seen` 枚，O(n²)，而
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
