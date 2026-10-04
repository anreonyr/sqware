use alloc::sync::Weak;

use env::{Mark, PieFail, PieToken};

use super::pie::{AnyPie, Heir, Need, Permission, new_pie};
use crate::work::room::messenger::{self, WakeKey};
use crate::work::unit::task::Task;

pub(crate) fn accord(
    caller: &Task,
    src: PieToken,
    dst: &Weak<Task>,
    subset: Permission,
    mark: Mark,
) -> Result<usize, PieFail> {
    let _graph = super::GRAPH.lock();
    let target = dst.upgrade().ok_or(PieFail::Denied)?;
    let mut operation = None;
    let granted = {
        let mut pies = caller.pies.lock();
        let pie = pies
            .iter_mut()
            .find(|p| p.token() == src)
            .ok_or(PieFail::Denied)?;
        if let AnyPie::Pole(p) = &*pie {
            operation = Some(p.meta().backing().operation().ok_or(PieFail::Busy)?);
            if p.meta().backing().reserved() != 0 {
                return Err(PieFail::HandedOver);
            }
        }
        if !pie.alive() {
            return Err(PieFail::Dead);
        }
        if !pie.allows(Need::Grant) {
            return Err(PieFail::Denied);
        }
        if !pie.covers(subset) {
            return Err(PieFail::Denied);
        }
        if !super::pie::form_ok(pie.permission(), subset) {
            return Err(PieFail::Denied);
        }
        if pie.heir().is_some() {
            return Err(PieFail::HandedOver);
        }
        // **含状态订阅的组不许转授**：订阅里有一格是"观察订阅者自己"，组一旦易主，
        // 那一格就与新持有者错配——不拒就会开出"借转授让别人的组替你观察"的口子。
        if let AnyPie::Tole(p) = &*pie
            && p.meta().has_subs()
        {
            return Err(PieFail::Denied);
        }
        let badge = if mark == Mark::NONE { pie.mark() } else { mark };
        let granted = match &*pie {
            AnyPie::Hole(p) => AnyPie::Hole(new_pie(p.meta().clone(), badge, subset, Some(src))),
            AnyPie::Pole(p) => AnyPie::Pole(super::try_new_pie(
                p.meta().clone(),
                badge,
                subset,
                Some(src),
            )?),
            AnyPie::Nole(p) => AnyPie::Nole(new_pie(p.meta().clone(), badge, subset, Some(src))),
            AnyPie::Tole(p) => AnyPie::Tole(new_pie(p.meta().clone(), badge, subset, Some(src))),
        };
        if pie.permission().contains(Permission::ONLY) {
            let h = Heir {
                task: target.ident.id,
                token: granted.token(),
            };
            match pie {
                AnyPie::Hole(p) => p.heir = Some(h),
                AnyPie::Pole(p) => p.heir = Some(h),
                AnyPie::Nole(p) => p.heir = Some(h),
                AnyPie::Tole(p) => p.heir = Some(h),
            }
        }
        granted
    };
    let token = granted.token();
    let mut kids = target.pies.lock();
    if kids.try_reserve(1).is_err() {
        drop(kids);
        drop(granted);
        clear_heir(caller, src);
        return Err(PieFail::OoM);
    }
    kids.push(granted);
    drop(kids);
    drop(operation);
    // 两处通知都必须在真实状态提交之后、且**出 `GRAPH`**：唤醒路径会 `kick` 到调度器锁，
    // 在这里发等于在 GRAPH 内制造一条新的跨锁关系。
    drop(_graph);
    let _ = messenger::wake(
        WakeKey::Pies {
            task: target.ident.id,
        },
        &target.life(),
    );
    // 外来 Accord 到达：目标的能力表变了。只要求复核，不代替任何判据；
    // 没有观察者时 `signal` 不留站点。
    let _ = messenger::signal(WakeKey::Capabilities {
        task: target.ident.id,
    });
    Ok(token.get())
}

pub(crate) fn clear_heir(task: &Task, token: PieToken) -> bool {
    let mut pies = task.pies.lock();
    let Some(pie) = pies.iter_mut().find(|p| p.token() == token) else {
        return false;
    };
    match pie {
        AnyPie::Hole(p) => p.heir.take().is_some(),
        AnyPie::Pole(p) => p.heir.take().is_some(),
        AnyPie::Nole(p) => p.heir.take().is_some(),
        AnyPie::Tole(p) => p.heir.take().is_some(),
    }
}
