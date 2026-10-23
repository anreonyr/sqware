use super::{Action, Directive, Origin, Request};
use crate::system::{app::Fault as Fail, control::unit::Control};
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;

pub(crate) struct Startup {
    plan: StartPlan,
    eligible: bool,
    waiting: Option<crate::system::assembly::JobId>,
}

struct StartPlan {
    units: Vec<&'static crate::unit::UnitFile>,
    next: usize,
}

impl Startup {
    pub(crate) fn new() -> Self {
        Self {
            plan: StartPlan {
                units: Vec::new(),
                next: 0,
            },
            eligible: false,
            waiting: None,
        }
    }
    pub(crate) fn configure(&mut self, list: Vec<&'static crate::unit::UnitFile>) {
        self.plan.units = list;
    }
    pub(crate) fn complete(&self) -> bool {
        self.plan.next == self.plan.units.len()
    }
    pub(crate) fn eligible(&self) -> bool {
        self.eligible
    }
}
pub fn startup(
    mut startup: ResMut<Startup>,
    mut operations: ResMut<crate::system::assembly::Jobs>,
    settle: Res<crate::system::app::policy::Settle>,
) -> Result<Progress, Fail> {
    if settle.flow.settling || settle.closing() || startup.plan.next == startup.plan.units.len() {
        return Ok(Progress::Done);
    }
    if let Some(id) = startup.waiting {
        let Some(result) = operations.take_local(id) else {
            return Ok(Progress::Done);
        };
        startup.waiting = None;
        let (completed, _) = result.map_err(|error| {
            crate::debug::put(&alloc::format!("system: startup failed {error:?}"));
            Fail::Shutdown
        })?;
        let action = match completed {
            Action::Mint => Action::Embark { parent: None },
            Action::Embark { .. } => {
                startup.plan.next += 1;
                Action::Mint
            }
            _ => return Ok(Progress::Done),
        };
        if let Some(program) = startup.plan.units.get(startup.plan.next) {
            startup.waiting = Some(
                operations
                    .submit_lifecycle(Request {
                        name: program.name().into(),
                        directive: Directive {
                            action,
                            origin: Origin::Startup,
                        },
                        back: None,
                    })
                    .map_err(|_| Fail::Room)?,
            );
        }
    } else if operations.is_empty() {
        startup.waiting = Some(
            operations
                .submit_lifecycle(Request {
                    name: startup.plan.units[startup.plan.next].name().into(),
                    directive: Directive {
                        action: Action::Mint,
                        origin: Origin::Startup,
                    },
                    back: None,
                })
                .map_err(|_| Fail::Room)?,
        );
    }
    Ok(Progress::Done)
}

pub fn eligibility(control: Res<Control>, mut startup: ResMut<Startup>) -> Result<Progress, Fail> {
    startup.eligible = startup.plan.next == startup.plan.units.len() && control.due();
    Ok(Progress::Done)
}
