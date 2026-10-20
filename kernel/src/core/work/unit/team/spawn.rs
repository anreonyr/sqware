use alloc::sync::Arc;
use alloc::vec::Vec;

use env::UnitFail;

use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::nest::NestError;
use crate::core::work::mail::pole::PoleOperation;
use crate::core::work::pend::{self, Permission};
use crate::core::work::unit::task::Task;

use super::Team;
use super::lease::StagingLease;

/// 往团队里落一个新任务：先把入口与栈定下，再把首次发布时随它一起交出去的暂存
/// 根（Pole 派生）从调用方的 pie 上摘下、换成新任务的成员与暂扣位。
///
/// `first` 之后的发布不再动暂存表：团队已经有默认入口了。
pub(crate) fn spawn(
    target: &Arc<Team>,
    caller: Option<&Arc<Task>>,
    entry: usize,
    words: Vec<usize>,
    stack: usize,
) -> Result<Arc<Task>, UnitFail> {
    fn map_err(error: NestError) -> UnitFail {
        if error == NestError::OutOfMemory {
            UnitFail::OoM
        } else {
            UnitFail::Denied
        }
    }
    let _construction = target.operation().ok_or(UnitFail::Busy)?;
    let first = !target.ready();
    let entry_va = if entry == 0 {
        target.default_entry()
    } else {
        entry
    };
    // 入口的前两个字节都要取得到：取不到就拒收。
    let entry = KVirt::from_raw(entry_va);
    if !entry_va.is_multiple_of(2)
        || target.space.instruction_byte(entry).is_none()
        || target.space.instruction_byte(entry + 1).is_none()
    {
        return Err(UnitFail::BadEntry);
    }
    let mut builder = target.task().entry(KVirt::from_raw(entry_va)).args(words);
    if stack > 0 {
        builder = builder.stack(stack);
    }
    let result = (|| -> Result<Arc<Task>, UnitFail> {
        let mut prepared = builder.prepare().map_err(map_err)?;
        if !first {
            return prepared.publish(|| {}).map_err(map_err);
        }
        let caller = caller.ok_or(UnitFail::Denied)?;
        let mut staged = StagingLease::hold(target);
        let mut operations = Vec::new();
        let mut retired = Vec::new();
        let mut leases = Vec::new();
        let mut sources = Vec::new();
        sources
            .try_reserve(staged.items.len())
            .map_err(|_| UnitFail::OoM)?;
        operations
            .try_reserve(staged.items.len())
            .map_err(|_| UnitFail::OoM)?;
        retired
            .try_reserve(staged.items.len())
            .map_err(|_| UnitFail::OoM)?;
        leases
            .try_reserve(staged.items.len())
            .map_err(|_| UnitFail::OoM)?;
        for item in staged.items.iter() {
            operations.push(PoleOperation::acquire(&item.meta).ok_or(UnitFail::Busy)?);
            let source = item.meta.authorization().ok_or(UnitFail::Denied)?;
            let (meta, token, _) = source.root().ok_or(UnitFail::Denied)?;
            if token != item.token || !Arc::ptr_eq(&meta, &item.meta) {
                return Err(UnitFail::Denied);
            }
            if sources.iter().any(|held| Arc::ptr_eq(held, &source)) {
                return Err(UnitFail::Denied);
            }
            sources.push(source);
        }
        let closed = caller.pend.lock();
        if !pend::live(caller) {
            return Err(UnitFail::Denied);
        }
        let mut pies = caller.pend.pies.lock();
        for item in staged.items.iter() {
            let Some(pie) = pies.iter().find(|p| p.token() == item.token) else {
                return Err(UnitFail::Denied);
            };
            let p = pie.snapshot().mail().pole().ok_or(UnitFail::Denied)?;
            if !Arc::ptr_eq(&p, &item.meta)
                || !p.alive()
                || pie.sire().is_some()
                || pie.heir().is_some()
                || p.owner() != caller.ident.id
                || !pie
                    .permission()
                    .contains(Permission::FETCH | Permission::VEST | Permission::ONLY)
                || p.reserved() != target.id.get()
                || p.mapped()
            {
                return Err(UnitFail::Denied);
            }
            let record = target
                .space
                .views
                .locate(item.view.identity())
                .ok_or(UnitFail::Denied)?;
            if !record
                .view()
                .is_some_and(|view| Arc::ptr_eq(view, &item.view))
                || !Arc::ptr_eq(
                    &record.source,
                    &item.meta.authorization().ok_or(UnitFail::Denied)?,
                )
            {
                return Err(UnitFail::Denied);
            }
            if record
                .control
                .as_ref()
                .is_some_and(|control| control.scope() == env::ViewControl::Caller)
            {
                prepared
                    .track_control(&target.space, item.view.identity())
                    .map_err(map_err)?;
            }
        }
        for (index, source) in sources.iter().enumerate() {
            if let Err(error) = source.pause_publish() {
                for paused in &sources[..index] {
                    paused.resume_publish();
                }
                return Err(if error == env::PieFail::Busy {
                    UnitFail::Busy
                } else {
                    UnitFail::Denied
                });
            }
        }
        let receiver = prepared.id();
        let result = prepared
            .publish(|| {
                for item in staged.items.drain(..) {
                    let index = pies
                        .iter()
                        .position(|p| p.token() == item.token)
                        .expect("staged root");
                    target.space.views.hand_over(item.view.identity(), receiver);
                    caller.release_control(item.view.identity());
                    let pie = pies.remove(index);
                    pie.invalidate();
                    caller
                        .pend
                        .heirs
                        .lock()
                        .retain(|(parent, _, _)| *parent != item.token);
                    retired.push(pie);
                    item.meta.detach_staging(item.token, target.id);
                    leases.push(item);
                }
                pend::changed(caller);
                target.set_default_entry(entry_va);
            })
            .map_err(map_err);
        if result.is_err() {
            for source in &sources {
                source.resume_publish();
            }
        }
        drop(pies);
        drop(closed);
        drop(staged);
        drop(operations);
        for pie in &retired {
            pend::notify(caller.ident.id, pie.token());
        }
        drop(retired);
        drop(leases);
        result
    })();
    result
}
