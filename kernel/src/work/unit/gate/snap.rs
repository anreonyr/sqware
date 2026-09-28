use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{PieToken, TaskId};

use crate::lock::OnceLock;
use crate::work::unit::task::Task;
use crate::work::unit::weak::TaskWeak;

use super::pie::AnyPie;

pub(crate) type Snap = [TaskWeak];

fn provider() -> &'static OnceLock<fn() -> Vec<TaskWeak>> {
    static P: OnceLock<fn() -> Vec<TaskWeak>> = OnceLock::new();
    &P
}

pub(crate) fn install(f: fn() -> Vec<TaskWeak>) {
    let _ = provider().set(f);
}

pub(crate) fn snap() -> Vec<TaskWeak> {
    provider().get().copied().map(|f| f()).unwrap_or_default()
}

#[cfg(debug_assertions)]
fn plausible(w: &Weak<Task>) -> bool {
    (Weak::as_ptr(w) as usize) >= 0x1000
}

#[cfg(not(debug_assertions))]
fn plausible(_w: &Weak<Task>) -> bool {
    true
}

pub(crate) fn find(tid: TaskId, snap: &Snap) -> Option<Arc<Task>> {
    for w in snap {
        if !plausible(w) {
            static BAD: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
            let n = BAD.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
            if n <= 8 {
                crate::putln!("snap::find: skip implausible weak ({n}) tid={}", tid.get());
            }
            continue;
        }
        if let Some(t) = w.upgrade()
            && t.ident.id == tid
        {
            return Some(t);
        }
    }
    None
}

pub(crate) fn vestor(pie: &AnyPie, snap: &Snap) -> Option<TaskId> {
    holder(pie.sire()?, snap)
}

fn holder(token: PieToken, snap: &Snap) -> Option<TaskId> {
    for w in snap {
        let Some(t) = w.upgrade() else { continue };
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

pub(crate) fn heirs(token: PieToken, snap: &Snap) -> Option<Vec<(Arc<Task>, PieToken)>> {
    let mut out: Vec<(Arc<Task>, PieToken)> = Vec::new();
    for w in snap {
        let Some(t) = w.upgrade() else { continue };
        let mut kids: Vec<PieToken> = Vec::new();
        {
            let pies = t.pies.lock();
            if kids.try_reserve(pies.len()).is_err() {
                return None;
            }
            kids.extend(
                pies.iter()
                    .filter(|p| p.sire() == Some(token))
                    .map(|p| p.token()),
            );
        }
        for k in kids {
            if out.try_reserve(1).is_err() {
                return None;
            }
            out.push((t.clone(), k));
        }
    }
    Some(out)
}