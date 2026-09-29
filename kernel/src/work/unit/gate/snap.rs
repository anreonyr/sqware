use alloc::sync::Arc;
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

/// 快照这一层**只剩"沿边找"这一件事**：找人（`find`）那一格退了——退场那一趟现在
/// 由钩子直接递来 `&Arc<Task>`，不必再从清册快照里把号找回来（原话见 `messenger::Hook`、
/// `gate::doom`）。留着它只会给"封印要先分配"那条路留个入口。
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
