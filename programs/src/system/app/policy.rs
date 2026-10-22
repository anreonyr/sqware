use crate::system::assembly::retirement::Participant;
use crate::system::app::Fault as Fail;
use crate::system::assembly::Jobs;
use crate::system::control::Startup;
use crate::system::control::unit::Control;

use crate::system::app::life::{Phase, Status};
use ::core::sync::atomic::Ordering;
use ::schedule::{Progress, Res, ResMut};
use alloc::sync::Arc;
use alloc::vec::Vec;
use env::Wait;

pub struct Flow {
    pub settling: bool,
    pub forced: bool,
    pub done: bool,
}
pub use super::activity::{Activity, activity};
pub struct Bound(pub Wait);
pub struct Shutoff(pub Option<u64>);

/// Retirement gate: every declared provider must report Done before settling.
pub struct Settle {
    pub flow: Flow,
    participants: Vec<Participant>,
    attempting: bool,
    closing: bool,
    domains: bool,
}

impl Settle {
    pub fn new(flow: Flow, programs: &[&crate::unit::UnitFile]) -> Self {
        let mut participants: Vec<Participant> = Vec::new();
        for program in programs {
            for declaration in program.control.retirement {
                if participants
                    .iter()
                    .any(|participant| participant.declaration == *declaration)
                {
                    continue;
                }
                participants.push(Participant::new(*declaration, env::TaskId::new(0)));
            }
        }
        Self {
            flow,
            participants,
            attempting: false,
            closing: false,
            domains: false,
        }
    }

    pub fn settled(&self) -> bool {
        self.participants.iter().all(|participant| participant.done)
    }

    pub(crate) fn settled_for(&self, declarations: &[crate::unit::Retirement]) -> bool {
        declarations.iter().all(|declaration| {
            self.participants
                .iter()
                .any(|participant| participant.declaration == *declaration && participant.done)
        })
    }

    pub(crate) fn working(&self) -> bool {
        self.attempting && !self.settled()
    }

    pub(crate) fn discover(
        &mut self,
        layout: Option<&crate::system::app::namespace::System>,
        namespaces: &crate::system::assembly::namespace::NamespaceInstallations,
    ) {
        let Some(system) = layout else {
            return;
        };
        for participant in &mut self.participants {
            participant.discover(system, namespaces);
        }
    }

    pub(crate) fn advance_all(&mut self) -> bool {
        self.attempting = true;
        for index in 0..self.participants.len() {
            self.advance_at(index);
        }
        self.settled()
    }

    pub(crate) fn advance(&mut self, declarations: &[crate::unit::Retirement]) -> bool {
        self.attempting = true;
        for index in 0..self.participants.len() {
            let declaration = self.participants[index].declaration;
            if declarations.contains(&declaration) {
                self.advance_at(index);
            }
        }
        self.settled_for(declarations)
    }

    pub(crate) fn gate(&mut self, program: &crate::unit::UnitFile) -> bool {
        let declarations = program.control.retirement;
        self.advance(declarations);
        self.settled_for(declarations)
    }

    fn advance_at(&mut self, index: usize) {
        let Some(participant) = self.participants.get_mut(index) else {
            return;
        };
        participant.advance();
    }

    fn enter(&mut self, forced: bool) -> bool {
        if !self.settled() {
            return false;
        }
        self.flow.settling = true;
        self.flow.forced |= forced;
        true
    }

    pub(crate) fn admit(&mut self, forced: bool) -> bool {
        self.advance_all();
        self.enter(forced)
    }

    /// Close new construction and stop dynamic execution domains before providers.
    pub(crate) fn close(&mut self, forced: bool) {
        if self.flow.settling {
            return;
        }
        self.closing = true;
        self.flow.forced |= forced;
    }

    pub(crate) fn closing(&self) -> bool {
        self.closing
    }

    /// Dynamic execution domains have all settled; the provider gate may now open.
    pub(crate) fn domains_settled(&mut self) {
        self.domains = true;
    }
}

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
/// 按装配声明发现退休面；缺失提供方保持未完成。
pub fn gate_open(
    layout: Res<crate::system::app::operator::Layout>,
    namespaces: Res<crate::system::assembly::namespace::NamespaceInstallations>,
    mut settle: ResMut<Settle>,
) -> Result<Progress, Fail> {
    settle.discover(layout.as_ref(), &namespaces);
    Ok(Progress::Done)
}

/// 达到退场条件后先停止动态执行域，全部结清后才推进退休提供方。
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
    if !settle.closing() {
        settle.close(false);
        return Ok(Progress::Done);
    }
    if settle.domains {
        settle.admit(false);
    }
    Ok(Progress::Done)
}
pub fn domains(control: Res<Control>, mut settle: ResMut<Settle>) -> Result<Progress, Fail> {
    if settle.closing()
        && !settle.flow.settling
        && control.instances().all(|item| item.settled())
    {
        settle.domains_settled();
    }
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
        // 超时退场与正常退场走同一道门：先停动态执行域，再等退休提供方。
        settle.close(true);
        programs::debug::put("system: forced shutdown closing dynamic execution domains");
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
        && control.instances().all(|item| item.settled());
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
    } else if settle.working() {
        Wait::AtMost(1)
    } else if settle.closing() && !settle.flow.settling {
        Wait::AtMost(1)
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
