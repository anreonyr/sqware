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
    let tokens: Vec<PieToken> = {
        let pies = task.pies.lock();
        let mut v: Vec<PieToken> = Vec::new();
        if v.try_reserve(pies.len()).is_err() {
            return;
        }
        v.extend(pies.iter().map(|p| p.token()));
        v
    };
    seal_owned(tid, &task);
    for token in tokens {
        cull((task.clone(), token), &snap);
    }
}

fn seal_owned(tid: TaskId, task: &Arc<Task>) -> usize {
    if tid.get() == 0 {
        return 0;
    }
    let owned: Vec<PieToken> = {
        let pies = task.pies.lock();
        let mut v: Vec<PieToken> = Vec::new();
        if v.try_reserve(pies.len()).is_err() {
            return 0;
        }
        v.extend(
            pies.iter()
                .filter(|p| p.owner_task() == tid)
                .map(|p| p.token()),
        );
        v
    };
    let mut sealed = 0;
    for token in owned {
        let meta = {
            let pies = task.pies.lock();
            pies.iter().find(|p| p.token() == token).map(|p| match p {
                AnyPie::Hole(h) => Resource::Hole(h.meta().clone()),
                AnyPie::Pole(pl) => Resource::Pole(pl.meta().clone()),
                AnyPie::Nole(n) => Resource::Nole(n.meta().clone()),
                AnyPie::Tole(t) => Resource::Tole(t.meta().clone()),
            })
        };
        match meta {
            Some(Resource::Hole(m)) => {
                hole::seal(&m);
                sealed += 1;
            }
            Some(Resource::Pole(m)) => {
                pole::seal(&m);
                sealed += 1;
            }
            Some(Resource::Nole(m)) => {
                nole::seal(&m);
                sealed += 1;
            }
            Some(Resource::Tole(m)) => {
                tole::seal(&m);
                sealed += 1;
            }
            None => {}
        }
    }
    sealed
}

enum Resource {
    Hole(Arc<HoleMeta>),
    Pole(Arc<PoleMeta>),
    Nole(Arc<NoleMeta>),
    Tole(Arc<ToleMeta>),
}