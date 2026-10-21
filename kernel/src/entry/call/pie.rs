use alloc::sync::Arc;

use env::{GrantRange, Mark, PieCall, PieFail, PieInfo, PieToken, ReleaseMode, TaskId, VirtAddr};

use crate::core::memory::address::{PageCount, PageOffset};
use crate::core::memory::address::VirtAddr as KVirt;
use crate::core::memory::nest::Access;
use crate::core::work::pend::{self, Permission, PieSnapshot};
use crate::core::work::room::conductor::current;
use crate::core::work::room::scheduler::roster::muster;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::TrapContext;

use super::gate;
use super::outcome::answer;

pub(crate) fn dispatch(frame: &mut TrapContext, call: PieCall, ident: Arc<TaskIdent>) {
    match call {
        PieCall::Accord {
            src,
            dst,
            subset,
            mark,
        } => accord(frame, src, dst, subset, mark),
        PieCall::Narrow { token, subset } => narrow(frame, token, subset),
        PieCall::Revoke { dst, token } => revoke(frame, dst, token),
        PieCall::Collect {
            after,
            buf,
            capacity,
        } => collect(frame, &ident, after, buf, capacity),
        PieCall::Inspect { token, buf } => inspect(frame, &ident, token, buf),
        PieCall::Release { token, mode } => release(frame, token, mode),
        PieCall::Same { a, b } => {
            answer(
                frame,
                current()
                    .running_task()
                    .ok_or(PieFail::Denied)
                    .and_then(|task| pend::same(&task, a, b))
                    .map(usize::from),
            );
        }
        PieCall::AccordWindow {
            src,
            dst,
            subset,
            mark,
            offset,
            count,
        } => accord_window(frame, src, dst, subset, mark, offset, count),
        PieCall::Derive {
            src,
            subset,
            mark,
            range,
        } => derive(frame, src, subset, mark, range),
    }
}

fn accord(
    frame: &mut TrapContext,
    src_token: PieToken,
    dst_id: TaskId,
    subset: Permission,
    mark: Mark,
) {
    let r = (|| -> Result<usize, PieFail> {
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let src = pend::locate(&caller, src_token).ok_or(PieFail::Denied)?;
        gate::usable(&src)?;
        let dst = muster(dst_id).ok_or(PieFail::Denied)?;
        pend::accord(&caller, src_token, dst.as_weak(), subset, mark)
    })();
    answer(frame, r);
}

/// 带有效窗口的 `Accord`：`offset`/`count` 用相对来源起点的绝对页说。
fn accord_window(
    frame: &mut TrapContext,
    src_token: PieToken,
    dst_id: TaskId,
    subset: Permission,
    mark: Mark,
    offset: usize,
    count: usize,
) {
    let r = (|| -> Result<usize, PieFail> {
        let count = PageCount::new(count).ok_or(PieFail::NotAligned)?;
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let src = pend::locate(&caller, src_token).ok_or(PieFail::Denied)?;
        gate::usable(&src)?;
        let dst = muster(dst_id).ok_or(PieFail::Denied)?;
        pend::accord_window(
            &caller,
            src_token,
            dst.as_weak(),
            subset,
            mark,
            Some((PageOffset::new(offset), count)),
        )
    })();
    answer(frame, r);
}

/// 本地精确派生：目标就是调用者，范围继承父窗口或按来源起点收窄。
fn derive(
    frame: &mut TrapContext,
    src_token: PieToken,
    subset: Permission,
    mark: Mark,
    range: GrantRange,
) {
    let r = (|| -> Result<usize, PieFail> {
        let window = match range {
            GrantRange::Inherit => None,
            GrantRange::Pages { offset, count } => {
                let count = PageCount::new(count).ok_or(PieFail::NotAligned)?;
                Some((PageOffset::new(offset), count))
            }
        };
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let src = pend::locate(&caller, src_token).ok_or(PieFail::Denied)?;
        gate::usable(&src)?;
        pend::derive(&caller, src_token, subset, mark, window)
    })();
    answer(frame, r);
}

fn narrow(frame: &mut TrapContext, token: PieToken, subset: Permission) {
    let r = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| pend::reduce(&task, token, subset))
        .map(|()| 0);
    answer(frame, r);
}

fn revoke(frame: &mut TrapContext, dst_id: TaskId, token: PieToken) {
    let r = (|| -> Result<usize, PieFail> {
        let caller = current().running_task().ok_or(PieFail::Denied)?;
        let target = muster(dst_id).ok_or(PieFail::Denied)?;
        pend::revoke(&caller, target.as_weak(), token).map(|_| 0)
    })();
    answer(frame, r);
}

fn information(task: &Arc<crate::core::work::unit::task::Task>, pie: &PieSnapshot) -> PieInfo {
    PieInfo {
        token: pie.token(),
        kind: pie.mail().kind(),
        permission: pie.permission(),
        owner: pie.mail().owner(),
        vestor: if pie.sire().is_some() {
            pend::vestor(task, pie.token()).unwrap_or(TaskId::new(0))
        } else {
            TaskId::new(0)
        },
        mark: pie.mark(),
        alive: pie.alive(),
    }
}

fn write_info(ident: &TaskIdent, buf: usize, info: PieInfo) -> Result<(), PieFail> {
    let words = info.words();
    let bytes =
        unsafe { ::core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), PieInfo::SIZE) };
    if !ident.team.space.copy_out(bytes, KVirt::wrap(buf)) {
        return Err(PieFail::Denied);
    }
    Ok(())
}

fn inspect(frame: &mut TrapContext, ident: &TaskIdent, token: PieToken, buf: VirtAddr) {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let pie = pend::locate(&task, token).ok_or(PieFail::Denied)?;
        if !ident
            .team
            .space
            .validate_access(KVirt::wrap(buf.get()), PieInfo::SIZE, Access::WRITE)
        {
            return Err(PieFail::Denied);
        }
        write_info(ident, buf.get(), information(&task, &pie))?;
        Ok(0)
    })();
    answer(frame, r);
}

fn collect(
    frame: &mut TrapContext,
    ident: &TaskIdent,
    after: PieToken,
    buf: VirtAddr,
    capacity: usize,
) {
    let r = (|| -> Result<usize, PieFail> {
        let task = current().running_task().ok_or(PieFail::Denied)?;
        let len = capacity.checked_mul(PieInfo::SIZE).ok_or(PieFail::Denied)?;
        if capacity == 0 {
            return Ok(0);
        }
        if !ident
            .team
            .space
            .validate_access(KVirt::wrap(buf.get()), len, Access::WRITE)
        {
            return Err(PieFail::Denied);
        }
        let capacity = capacity.min(16);
        let mut snapshots: [Option<PieSnapshot>; 16] = ::core::array::from_fn(|_| None);
        let mut count = 0;
        {
            let table = task.pend.pies.lock();
            for pie in table.iter().filter(|p| p.token().get() > after.get()) {
                let at = snapshots[..count]
                    .partition_point(|p| p.as_ref().unwrap().token().get() < pie.token().get());
                if at < capacity {
                    let next = (count + 1).min(capacity);
                    for i in (at + 1..next).rev() {
                        snapshots[i] = snapshots[i - 1].take();
                    }
                    snapshots[at] = Some(pie.snapshot());
                    count = next;
                }
            }
        }
        for (i, pie) in snapshots[..count].iter().enumerate() {
            write_info(
                ident,
                buf.get() + i * PieInfo::SIZE,
                information(&task, pie.as_ref().unwrap()),
            )?;
        }
        Ok(count)
    })();
    answer(frame, r);
}

fn release(frame: &mut TrapContext, token: PieToken, mode: ReleaseMode) {
    let r = current()
        .running_task()
        .ok_or(PieFail::Denied)
        .and_then(|task| match mode {
            ReleaseMode::Revoke => pend::release(&task, token).map(|_| ()),
            ReleaseMode::Keep => pend::forget(&task, token),
        })
        .map(|_| 0);
    answer(frame, r);
}
