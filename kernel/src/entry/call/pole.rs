use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{
    Bit, FlatReference, Mark, PieFail, PieKind, PieToken, PoleCall, PoleCondition, PoleFail, Wait,
};

use super::gate;
use super::outcome::{Outcome, answer, answer_pair, ret_err};
use crate::core::memory::address::PageOffset;
use crate::core::memory::nest::Access;
use crate::core::work::mail::{self, Mail};
use crate::core::work::pend::{
    self, Derivation, Geometry, Hold, Lineage, Need, Permission, SourceAuthorization,
};
use crate::core::work::room::conductor::current;
use crate::core::work::room::messenger::Handoff;
use crate::core::work::unit::task::TaskIdent;
use crate::machine::switcher::context::{Gprs, TrapContext};

pub(super) fn dispatch(frame: &mut TrapContext, call: PoleCall, ident: Arc<TaskIdent>) -> Outcome {
    match call {
        PoleCall::Unseal { reference, shared } => {
            let result = current()
                .running_task()
                .ok_or(PieFail::Denied)
                .and_then(|task| create_pole(&task, reference, shared));
            answer(frame, result.map(|token| token.get()).map_err(fail));
        }
        PoleCall::Seal { token } => {
            answer(
                frame,
                super::seal::seal_resource(token, PieKind::Pole).map_err(fail),
            );
        }
        PoleCall::Open { token } => {
            answer_pair(frame, open_resource(ident, token).map_err(fail));
        }
        PoleCall::Shut { token } => {
            answer(frame, shut_resource(token).map_err(fail));
        }
        PoleCall::Wait {
            token,
            condition,
            millis,
        } => {
            let PoleCondition::Signal(bit) = condition;
            return wait(frame, ident, token, bit, millis);
        }
        PoleCall::Ring { token, bit } => return ring(frame, token, bit),
        PoleCall::Hush { token, bit } => return hush(frame, token, bit),
    }
    Outcome::Resume
}

fn wait(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    bit: Bit,
    millis: Wait,
) -> Outcome {
    let dur = millis.into_duration();
    let r = with_pole(token, Need::Fetch, |meta| {
        frame.gpr.set_x(Gprs::A0, 0);
        drop(ident);
        mail::pole::wait(meta, bit, dur)
    });
    match r {
        Ok(Handoff::Resume(ready)) => frame.gpr.set_x(Gprs::A0, ready as usize),
        Ok(Handoff::Switch(pa)) => return Outcome::Switch(pa as *mut TrapContext),
        Err(e) => ret_err(frame, e),
    }
    Outcome::Resume
}

fn ring(frame: &mut TrapContext, token: PieToken, bit: Bit) -> Outcome {
    let r = with_pole(token, Need::Store, |meta| mail::pole::ring(meta, bit));
    answer(frame, r.map(|()| 0));
    Outcome::Resume
}

fn hush(frame: &mut TrapContext, token: PieToken, bit: Bit) -> Outcome {
    let r = with_pole(token, Need::Fetch, |meta| mail::pole::hush(meta, bit));
    answer(frame, r.map(|()| 0));
    Outcome::Resume
}

fn with_pole<T>(
    token: PieToken,
    need: Need,
    operation: impl FnOnce(&Arc<mail::pole::PoleMeta>) -> Result<T, PoleFail>,
) -> Result<T, PoleFail> {
    let pie = gate::accede(token, need).map_err(fail)?;
    gate::usable(&pie).map_err(fail)?;
    let meta = pie.mail().pole().ok_or(PoleFail::Denied)?;
    operation(&meta)
}

fn subset_to_access(subset: Permission) -> Result<Access, PieFail> {
    if !subset.contains(Permission::FETCH) {
        return Err(PieFail::Denied);
    }
    let mut access = Access::READ;
    if subset.contains(Permission::STORE) {
        access |= Access::WRITE;
    }
    Ok(access)
}

#[cfg(debug_assertions)]
static FAIL_POLE: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);

#[cfg(debug_assertions)]
pub(crate) fn fail_pole_at(stage: usize) {
    assert!(crate::testing());
    FAIL_POLE.store(stage, ::core::sync::atomic::Ordering::Relaxed);
}

fn pole_checkpoint(_stage: usize) -> Result<(), PieFail> {
    #[cfg(debug_assertions)]
    if FAIL_POLE
        .compare_exchange(
            _stage,
            0,
            ::core::sync::atomic::Ordering::Relaxed,
            ::core::sync::atomic::Ordering::Relaxed,
        )
        .is_ok()
    {
        return Err(PieFail::OoM);
    }
    Ok(())
}

pub(crate) fn create_pole(
    task: &Arc<crate::core::work::unit::task::Task>,
    reference: FlatReference,
    shared: bool,
) -> Result<PieToken, PieFail> {
    let _serial = task.pend.lock();
    if !pend::live(task) {
        return Err(PieFail::Dead);
    }
    let hold = task
        .pend
        .references
        .get(reference.get())
        .ok_or(PieFail::Denied)?;
    let authorization = hold.source().clone();
    // 已交给根 Grant 的来源没有私有阶段：只能按**子段引用**派生一枚独立子资源。
    let Some(source) = authorization.own_range() else {
        return derive_pole(task, &hold, &authorization, shared);
    };
    // 只能从**私有根**铸新根：子 Pie / Pole 派生 / 借用都不产生新根资格。
    let (offset, count) = hold.window();
    if !authorization.private_owner(task.ident.id)
        || hold.retains()
        || hold.branch().is_some()
        || offset.get() != 0
        || count != source.page_count()
    {
        return Err(PieFail::Denied);
    }
    // 一次性创建资格：单次 claim，竞争方 Busy、已消费 Denied。
    authorization.claim()?;
    let result = (|| {
        pole_checkpoint(1)?;
        let size = source
            .page_count()
            .checked_bytes()
            .ok_or(PieFail::NotAligned)?;
        let meta = mail::pole::PoleMeta::from_source(source.clone(), size, task.ident.id)?;
        let mut permission = Permission::FETCH | Permission::STORE | Permission::VEST;
        if !shared {
            permission |= Permission::ONLY;
        }
        let pie = pend::Pie::try_new(Mail::Pole(meta.clone()), Mark::NONE, permission, None)?;
        let token = pie.token();
        let mut inputs = Vec::new();
        for space in authorization.spaces()? {
            let entries = space.views.owned_by(&authorization)?;
            inputs
                .try_reserve(entries.len())
                .map_err(|_| PieFail::OoM)?;
            for entry in entries {
                if let Some(view) = entry.view() {
                    inputs.push((space.clone(), view.clone()));
                }
            }
        }
        meta.reserve_records(inputs.len())?;
        task.pend
            .pies
            .lock()
            .try_reserve(1)
            .map_err(|_| PieFail::OoM)?;
        pole_checkpoint(2)?;

        let _commit = crate::core::work::commit();
        let _publication = authorization.begin_withdraw()?;
        if !pend::live(task) || !authorization.private_owner(task.ident.id) {
            return Err(PieFail::Denied);
        }
        pole_checkpoint(3)?;
        meta.attach_source(&authorization);
        authorization.adopt(&meta, token, pie.grant().clone())?;
        let transition = (|| {
            pole_checkpoint(4)?;
            for (space, view) in inputs {
                meta.adopt_input(token, &space, view);
            }
            pole_checkpoint(5)?;
            let mut pies = task.pend.pies.lock();
            assert!(pies.len() < pies.capacity(), "reserved root Pie slot");
            meta.commit_source();
            authorization.consume();
            pies.push(pie);
            pend::changed(task);
            Ok(token)
        })();
        if transition.is_err() {
            authorization.restore_private(source.clone());
        }
        transition
    })();
    if result.is_err() {
        authorization.release_claim();
    }
    result
}

/// 从已接管的来源派生一枚**独立子资源**：它有自己的根 Pie 与自己的来源记录。
///
/// 父分支仍活着（FETCH + VEST）、请求落在这一枚分支的有效窗口里、子段是父资源的
/// 严格真子段、形态与父一致，且父侧没有别的子段与它重叠——任一不满足就整体拒绝。
/// 子记录生来就在根相位，因此发不出新的私有创建资格；来源依赖走子记录强持父记录，
/// 不用 Pie 的 sire/heirs 表达。
fn derive_pole(
    task: &Arc<crate::core::work::unit::task::Task>,
    hold: &Arc<Hold>,
    parent: &Arc<SourceAuthorization>,
    shared: bool,
) -> Result<PieToken, PieFail> {
    // 授权面：只认这枚引用所在的那一条活分支。
    let (branch, grant) = hold.branch().ok_or(PieFail::Denied)?;
    let permission = grant.permission().ok_or(PieFail::Dead)?;
    if !permission.contains(Permission::FETCH | Permission::VEST) {
        return Err(PieFail::Denied);
    }
    // 独立保留是这一段的外部持有，不是可再授权的来源。
    if hold.retains() {
        return Err(PieFail::Denied);
    }
    // 范围面：请求是相对父资源起点的页窗口，换本地绝对页号只做一次。
    let (offset, count) = hold.window();
    let parent_range = parent.full_range().ok_or(PieFail::Denied)?;
    let outer = grant
        .window()
        .unwrap_or((PageOffset::ZERO, parent_range.page_count()));
    let outer_end = outer
        .0
        .get()
        .checked_add(outer.1.get())
        .ok_or(PieFail::NotAligned)?;
    let request_end = offset
        .get()
        .checked_add(count.get())
        .ok_or(PieFail::NotAligned)?;
    if offset.get() < outer.0.get() || request_end > outer_end {
        return Err(PieFail::Denied);
    }
    let child_range = parent_range
        .range(offset, count)
        .map_err(|_| PieFail::Denied)?;
    // 严格真子段：整段父资源不从这里派生。
    if offset.get() == 0 && count == parent_range.page_count() {
        return Err(PieFail::Denied);
    }
    // 形态面：ONLY 位跟父分支一致；子段只拿父分支能给的位，一位也不多给。
    if permission.contains(Permission::ONLY) == shared {
        return Err(PieFail::Denied);
    }
    let child_permission =
        permission & (Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY);
    // 并列占用：父侧已有子段或别人的独立持有与它相交就整体拒绝。
    // 建新授权不享受“持有人自己”那一档，任何独立持有都照挡。
    // 可失败的准备全部在发表之前：几何、子 Pole、子 Pie 与子记录。
    let size = count.checked_bytes().ok_or(PieFail::NotAligned)?;
    let geometry = Geometry::new(offset, count)?;
    let meta = mail::pole::PoleMeta::from_source(child_range, size, task.ident.id)?;
    let pie = pend::Pie::try_new(Mail::Pole(meta.clone()), Mark::NONE, child_permission, None)?;
    let token = pie.token();
    let child = SourceAuthorization::derived(
        &meta,
        token,
        pie.grant().clone(),
        parent.ceiling(),
        task.ident.id,
        Lineage::new(parent.clone(), geometry.clone()),
    )?;
    let derivation = Derivation::new(branch, geometry, Arc::downgrade(&child));
    pole_checkpoint(6)?;
    // 准入复核、容量准备与发表同在一个 `commit` 段：这一段之后不再失败。
    let _commit = crate::core::work::commit();
    if !pend::live(task)
        || grant.permission().is_none()
        || parent.revoked()
        || parent.root().is_none_or(|(meta, _, _)| !meta.alive())
    {
        return Err(PieFail::Denied);
    }
    parent.check_publish(offset, count, None)?;
    parent.reserve_derivation()?;
    task.pend
        .pies
        .lock()
        .try_reserve(1)
        .map_err(|_| PieFail::OoM)?;
    // 槽先于 Pie 发表：并发使用要么看见槽（拒绝），要么看不见（这一段本来就空）。
    meta.attach_source(&child);
    parent.publish_derivation(derivation);
    task.pend.pies.lock().push(pie);
    pend::changed(task);
    Ok(token)
}

fn open_resource(ident: Arc<TaskIdent>, token: PieToken) -> Result<(usize, usize), PieFail> {
    let pie = gate::accede(token, Need::Fetch)?;
    gate::usable(&pie)?;
    let p = pie.mail().pole().ok_or(PieFail::Denied)?;
    let access = subset_to_access(pie.permission())?;
    mail::pole::open(&p, token, &ident.team.space, access)
}

fn shut_resource(token: PieToken) -> Result<usize, PieFail> {
    let task = current().running_task().ok_or(PieFail::Denied)?;
    let pie = pend::locate(&task, token).ok_or(PieFail::Denied)?;
    if !pie.allows(Need::Fetch) {
        return Err(PieFail::Denied);
    }
    gate::usable(&pie)?;
    let p = pie.mail().pole().ok_or(PieFail::Denied)?;
    mail::pole::shut(&p, token).map(|()| 0)
}

fn fail(error: PieFail) -> PoleFail {
    match error {
        PieFail::Denied => PoleFail::Denied,
        PieFail::NotAligned => PoleFail::NotAligned,
        PieFail::Dead => PoleFail::Dead,
        PieFail::OoM => PoleFail::OoM,
        PieFail::HandedOver => PoleFail::HandedOver,
        PieFail::Busy => PoleFail::Busy,
    }
}
