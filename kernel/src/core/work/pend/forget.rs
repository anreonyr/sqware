use alloc::sync::Arc;

use crate::core::work::unit::task::Task;
use env::{Permission, PieFail, PieToken};

pub(crate) fn forget(task: &Arc<Task>, token: PieToken) -> Result<(), PieFail> {
    for _ in 0..super::RETRIES {
        if super::locate(task, token).is_none() {
            return Err(PieFail::Denied);
        }
        let (tasks, nodes) = match super::cull::collect(task, token, false) {
            Err(PieFail::Busy) => continue,
            result => result?,
        };
        let result = super::with_tasks(&tasks, || {
            let pie = super::locate(task, token).ok_or(PieFail::Denied)?;
            if pie.permission().contains(Permission::ONLY) {
                return Err(PieFail::Denied);
            }
            let parent = pie.sire().ok_or(PieFail::Denied)?;
            let lord = pie.lord().upgrade().ok_or(PieFail::Denied)?;
            let replacement = super::locate(&lord, parent).ok_or(PieFail::Denied)?;
            if !super::live(&lord) || !replacement.mail().same(pie.mail()) {
                return Err(PieFail::Denied);
            }
            // 已封闭资源只移除引用；保留的后代不会因此恢复可用授权。
            if pie.mail().alive() {
                if !replacement.alive() {
                    return Err(PieFail::Denied);
                }
                for (holder, child_token) in nodes.iter().skip(1) {
                    let child = super::locate(holder, *child_token).ok_or(PieFail::Denied)?;
                    if !replacement.permission().contains(child.permission()) {
                        return Err(PieFail::Denied);
                    }
                }
            }
            // 可失败的准备全部在首写之前：heirs 容量先备，再取来源侧那次快照。
            // 两处任一失败都还没写过任何索引或父子对，整条忘了这条路可以原样重试。
            lord.pend
                .heirs
                .lock()
                .try_reserve(nodes.len() - 1)
                .map_err(|_| PieFail::OoM)?;
            // 历史来源槽与显式控制记录里的分支索引在同一事务里改指：
            // 权限仍是创建时的快照，不重算、也不部分复制。
            if let Some(meta) = pie.mail().pole()
                && let Some(authorization) = meta.authorization()
            {
                authorization.retarget_branch(token, parent)?;
            }
            for (child, child_token) in nodes.iter().skip(1) {
                let mut pies = child.pend.pies.lock();
                let pie = pies
                    .iter_mut()
                    .find(|p| p.token() == *child_token)
                    .expect("locked child");
                pie.parent(parent, Arc::downgrade(&lord));
                drop(pies);
                super::insert_heir(&lord, parent, Arc::downgrade(child), *child_token);
                super::changed(child);
            }
            let removed = super::cull::take(task, token).expect("locked forgotten token");
            Ok(removed)
        });
        let removed = match result {
            Err(PieFail::Busy) => {
                core::hint::spin_loop();
                continue;
            }
            result => result??,
        };
        if let Some(p) = removed.snapshot().mail().pole() {
            let _ = crate::core::work::mail::pole::shut(&p, token);
        }
        super::notify(task.ident.id, token);
        for (child, child_token) in nodes.iter().skip(1) {
            super::notify(child.ident.id, *child_token);
        }
        return Ok(());
    }
    Err(PieFail::Busy)
}

pub(crate) fn same(task: &Arc<Task>, a: PieToken, b: PieToken) -> Result<bool, PieFail> {
    let pies = task.pend.pies.lock();
    let find = |token| {
        pies.iter()
            .find(|p| p.token() == token)
            .ok_or(PieFail::Denied)
    };
    let (a, b) = (find(a)?.snapshot(), find(b)?.snapshot());
    if !a.alive() || !b.alive() {
        return Err(PieFail::Dead);
    }
    Ok(a.mail().same(b.mail()))
}
