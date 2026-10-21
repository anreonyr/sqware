use crate::system::app::Fault as Fail;
use crate::system::assembly::Jobs;
use crate::system::control::Startup;
use crate::system::control::unit::Control;

use crate::system::app::life::{Phase, Status};
use ::core::sync::atomic::Ordering;
use ::schedule::{Progress, Res, ResMut};
use alloc::sync::Arc;
use env::Wait;

pub struct Flow {
    pub settling: bool,
    pub forced: bool,
    pub done: bool,
}
pub use super::activity::{Activity, activity};
pub struct Bound(pub Wait);
pub struct Shutoff(pub Option<u64>);

/// 退场状态：流动那一份 ＋ **Hub 结清门**。
///
/// 门核对 α 启动/退休责任与 DMA 绑定均已结清（`open == Some(0)`）。
/// 正常退场与超时退场都要先过这道门，才允许 `flow.settling` 触发 Control 的
/// Slay/Ruin；没结清就一直停在门口，责任保留在 Hub 与各驱动那一侧，
/// 本机不因此假报已停。
pub struct Settle {
    pub flow: Flow,
    hub: Option<crate::driver::shared::dma::HubDma>,
    open: Option<u8>,
}

impl Settle {
    pub fn new(flow: Flow) -> Self {
        Self {
            flow,
            hub: None,
            open: None,
        }
    }

    /// 结清了吗（没问过或没结清 ⇒ false）。
    pub fn settled(&self) -> bool {
        self.open == Some(0)
    }

    /// 向 Hub 提一句“结清”（幂等：已在收场的继续同一责任），并把还有几笔记下来。
    pub(crate) fn ask(&mut self) {
        let Some(hub) = self.hub.as_ref() else {
            self.open = None;
            return;
        };
        self.open = hub.settle(Wait::AtMost(MS)).ok();
    }
}

/// 等 Hub 那一面的上限（毫秒）。
const MS: usize = 1000;

pub fn health(status: Res<Arc<Status>>) -> Result<Progress, Fail> {
    for id in [
        status.operator.load(Ordering::Acquire),
        status.identity.load(Ordering::Acquire),
    ] {
        if env::unit::join_task(env::TaskId::new(id), Wait::POLL).unwrap_or(true) {
            return Err(Fail::Dead);
        }
    }
    Ok(Progress::Done)
}
/// 找 Hub 那一面（找到就留着；没找到下一格再来）。
pub fn gate_open(
    layout: Res<crate::system::app::operator::Layout>,
    namespaces: Res<crate::system::assembly::namespace::NamespaceInstallations>,
    mut settle: ResMut<Settle>,
) -> Result<Progress, Fail> {
    if settle.hub.is_some() {
        return Ok(Progress::Done);
    }
    let Some(system) = layout.as_ref() else {
        return Ok(Progress::Done);
    };
    let capability = system.capability();
    let Some(hub) = namespaces.program_grant("hub", 0) else {
        return Ok(Progress::Done);
    };
    let Ok(dma) = capability.find(
        hub.entry,
        "dma",
        system_api::operator::Permit::Public,
        Wait::AtMost(MS),
    ) else {
        return Ok(Progress::Done);
    };
    if let Ok(client) = crate::driver::shared::dma::HubDma::open(dma.resource) {
        settle.hub = Some(client);
    }
    Ok(Progress::Done)
}

/// 退场门：够条件退场时先让 Hub 结清，结清之后才允许 `settling`。
pub fn settle(
    startup: Res<Startup>,
    operations: Res<Jobs>,
    mut settle: ResMut<Settle>,
) -> Result<Progress, Fail> {
    if settle.flow.settling {
        return Ok(Progress::Done);
    }
    if !(startup.eligible() && operations.is_empty()) {
        return Ok(Progress::Done);
    }
    if settle.settled() {
        settle.flow.settling = true;
        return Ok(Progress::Done);
    }
    settle.ask();
    Ok(Progress::Done)
}
pub fn deadline(settle: Res<Settle>, mut shutoff: ResMut<Shutoff>) -> Result<Progress, Fail> {
    if settle.flow.settling && shutoff.0.is_none() {
        shutoff.0 = Some(env::chrono::clock());
    }
    Ok(Progress::Done)
}
pub fn idle(
    activity: Res<Activity>,
    mut settle: ResMut<Settle>,
    shutoff: Res<Shutoff>,
) -> Result<Progress, Fail> {
    let now = env::chrono::clock();
    if settle.flow.settling {
        if shutoff
            .0
            .is_some_and(|since| now.saturating_sub(since) >= 10_000_000_000)
        {
            return Err(Fail::Idle);
        }
    } else if activity.walking && now.saturating_sub(activity.quiet) >= 10_000_000_000 {
        // 超时退场与正常退场走同一道门：没结清就不进 settling。
        if settle.settled() {
            programs::debug::put("system: forced shutdown admitted after DMA settlement");
            settle.flow.forced = true;
            settle.flow.settling = true;
        } else {
            programs::debug::put("system: idle deadline reached; awaiting DMA settlement");
            settle.ask();
        }
    }
    Ok(Progress::Done)
}
pub fn done(
    control: Res<Control>,
    operations: Res<Jobs>,
    mut settle: ResMut<Settle>,
) -> Result<Progress, Fail> {
    settle.flow.done = settle.flow.settling
        && control.done()
        && operations.is_empty()
        && control.instances().all(|item| item.team.is_none());
    if settle.flow.done && settle.flow.forced {
        return Err(Fail::Idle);
    }
    Ok(Progress::Done)
}
pub fn bound(
    activity: Res<Activity>,
    settle: Res<Settle>,
    mut bound: ResMut<Bound>,
) -> Result<Progress, Fail> {
    bound.0 = if settle.flow.done {
        Wait::POLL
    } else if !settle.flow.settling && !activity.walking {
        Wait::Forever
    } else {
        Wait::AtMost(
            (10_000_000_000u64.saturating_sub(env::chrono::clock().saturating_sub(activity.quiet)))
                .div_ceil(1_000_000)
                .max(1) as usize,
        )
    };
    Ok(Progress::Done)
}

pub fn running(
    startup: Res<Startup>,
    operations: Res<Jobs>,
    status: Res<Arc<Status>>,
) -> Result<Progress, Fail> {
    if startup.complete()
        && operations.is_empty()
        && status.phase.load(Ordering::Acquire) == Phase::Starting as u8
    {
        status.phase.store(Phase::Running as u8, Ordering::Release);
        programs::debug::put("system: running");
    }
    Ok(Progress::Done)
}

pub fn stopping_bound(
    settle: Res<Settle>,
    shutoff: Res<Shutoff>,
    mut bound: ResMut<Bound>,
) -> Result<Progress, Fail> {
    if settle.flow.settling && !settle.flow.done {
        let left = shutoff.0.map_or(1_000_000, |since| {
            10_000_000_000u64.saturating_sub(env::chrono::clock().saturating_sub(since))
        });
        bound.0 = Wait::AtMost(left.div_ceil(1_000_000).max(1) as usize);
    }
    Ok(Progress::Done)
}
