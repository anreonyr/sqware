use alloc::sync::{Arc, Weak};

use env::{Mark, PieFail, PieToken};

use super::Need;
use super::pie::{Heir, Permission, PieSnapshot};
use crate::core::memory::address::{PageCount, PageOffset};
use crate::core::work::grant::narrows;
use crate::core::work::mail::pole::PoleOperation;
use crate::core::work::room::messenger::{self, WakeKey};
use crate::core::work::unit::task::Task;

/// 这一枚子分支的有效窗口。
///
/// 不请求就**继承**父的那一段（带窗口的父派生出来的子仍带那一段，不是整段来源）；
/// 请求的那一段必须落在父的有效窗口里（父没有窗口就是整段来源），越窗与溢出一律拒。
/// ABI 解码检查非空与溢出；这一层检查父范围。
/// 窗口只在 Pole（有真实来源）上成立。
fn effective_window(
    pie: &PieSnapshot,
    requested: Option<(PageOffset, PageCount)>,
) -> Result<Option<(PageOffset, PageCount)>, PieFail> {
    let Some(requested) = requested else {
        return Ok(pie.grant().window());
    };
    let meta = pie.mail().pole().ok_or(PieFail::Denied)?;
    let outer = pie
        .grant()
        .window()
        .unwrap_or((PageOffset::ZERO, meta.source().page_count()));
    let outer_end = outer.0.get().checked_add(outer.1.get());
    let inner_end = requested.0.get().checked_add(requested.1.get());
    let inside = matches!(
        (outer_end, inner_end),
        (Some(outer_end), Some(inner_end))
            if requested.0.get() >= outer.0.get() && inner_end <= outer_end
    );
    if !inside {
        return Err(PieFail::Denied);
    }
    Ok(Some(requested))
}

pub(crate) fn accord(
    caller: &Arc<Task>,
    src: PieToken,
    dst: &Weak<Task>,
    subset: Permission,
    mark: Mark,
) -> Result<usize, PieFail> {
    publish(caller, src, dst, subset, mark, None)
}

/// 本地派生：目标就是 `caller`，范围继承或收窄，仍走同一条授权发布实现。
pub(crate) fn derive(
    caller: &Arc<Task>,
    src: PieToken,
    subset: Permission,
    mark: Mark,
    window: Option<(PageOffset, PageCount)>,
) -> Result<usize, PieFail> {
    publish(caller, src, &Arc::downgrade(caller), subset, mark, window)
}

/// 两条授权路径的唯一发布后端：`accord` 继承父窗口，`derive` 可显式收窄。
///
/// 请求的那一段用**相对来源起点的绝对页偏移**说（不是相对父分支的偏移），
/// 且必须落在父的有效窗口里：重复与嵌套派生只能单调收窄，没有放宽的路。
fn publish(
    caller: &Arc<Task>,
    src: PieToken,
    dst: &Weak<Task>,
    subset: Permission,
    mark: Mark,
    requested: Option<(PageOffset, PageCount)>,
) -> Result<usize, PieFail> {
    let target = dst.upgrade().ok_or(PieFail::Denied)?;
    let token = super::with_pair(caller, &target, || {
        if !super::live(caller) || !super::live(&target) {
            return Err(PieFail::Dead);
        }
        let pie = super::locate(caller, src).ok_or(PieFail::Denied)?;
        let pole = pie.mail().pole();
        let _operation = if let Some(p) = &pole {
            let operation = PoleOperation::acquire(p).ok_or(PieFail::Busy)?;
            if p.reserved() != 0 {
                return Err(PieFail::HandedOver);
            }
            Some(operation)
        } else {
            None
        };
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if !pie.allows(Need::Vest) || !narrows(pie.permission(), subset) {
            return Err(PieFail::Denied);
        }
        if pie.heir().is_some() {
            return Err(PieFail::HandedOver);
        }
        if pie.mail().tole().is_some_and(|p| p.has_cells()) {
            return Err(PieFail::Denied);
        }
        let mark = if mark == Mark::NONE { pie.mark() } else { mark };
        let window = effective_window(&pie, requested)?;
        // 授权发布操作的是这一段窗口：这一段的子段或别人的独立持有仍在占用就不建
        // 新分支，也不隐式裁到剩余范围（建新授权不享受“持有人自己”那一档）。
        if let Some(meta) = &pole
            && let Some(authorization) = meta.authorization()
        {
            let (offset, count) = window.unwrap_or((PageOffset::ZERO, meta.source().page_count()));
            authorization.check_publish(offset, count, None)?;
        }
        let mut granted = pie.vest_window(mark, subset, window)?;
        granted.parent(src, Arc::downgrade(caller));
        caller
            .pend
            .heirs
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        target
            .pend
            .pies
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        let token = granted.token();
        super::insert_heir(caller, src, Arc::downgrade(&target), token);
        if pie.permission().contains(Permission::ONLY) {
            let mut pies = caller.pend.pies.lock();
            let source = pies
                .iter_mut()
                .find(|p| p.token() == src)
                .expect("locked source");
            let heir = Heir {
                task: target.ident.id,
                token,
            };
            source.set_heir(Some(heir));
        }
        target.pend.pies.lock().push(granted);
        super::changed(caller);
        if !Arc::ptr_eq(caller, &target) {
            super::changed(&target);
        }
        Ok(token)
    })?;
    let _ = messenger::wake(
        WakeKey::Pies {
            task: target.ident.id,
        },
        &target.life(),
    );
    super::notify(caller.ident.id, src);
    super::notify(target.ident.id, token);
    Ok(token.get())
}

pub(crate) fn clear_heir(task: &Task, token: PieToken, expected: Heir) -> bool {
    let pend_guard = task.pend.lock();
    let commit_guard = crate::core::work::commit();
    let child = {
        let heirs = task.pend.heirs.lock();
        heirs
            .iter()
            .find(|(parent, _, child)| *parent == token && *child == expected.token)
            .and_then(|(_, task, _)| task.upgrade())
    };
    if child.is_some_and(|child| super::locate(&child, expected.token).is_some()) {
        return false;
    }
    let cleared = clear_heir_locked(task, token, expected);
    if cleared {
        super::remove_heir(task, token, expected.token);
    }
    drop(commit_guard);
    drop(pend_guard);
    if cleared {
        super::notify(task.ident.id, token);
    }
    cleared
}

pub(super) fn clear_heir_locked(task: &Task, token: PieToken, expected: Heir) -> bool {
    let mut pies = task.pend.pies.lock();
    let Some(pie) = pies.iter_mut().find(|p| p.token() == token) else {
        return false;
    };
    if pie.heir() != Some(&expected) {
        return false;
    }
    pie.set_heir(None);
    super::changed(task);
    true
}
