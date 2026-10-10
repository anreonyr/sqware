use super::{Action, Operations};
use crate::system::{app::Fault as Fail, control::unit::Control};
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;

pub(crate) struct Startup {
    list: Vec<&'static crate::unit::UnitFile>,
    at: usize,
    eligible: bool,
}

impl Startup {
    pub(crate) fn new() -> Self {
        Self {
            list: Vec::new(),
            at: 0,
            eligible: false,
        }
    }
    pub(crate) fn configure(&mut self, list: Vec<&'static crate::unit::UnitFile>) {
        self.list = list;
    }
    pub(crate) fn complete(&self) -> bool {
        self.at == self.list.len()
    }
    pub(crate) fn eligible(&self) -> bool {
        self.eligible
    }
}
pub fn startup(
    mut startup: ResMut<Startup>,
    mut operations: ResMut<Operations>,
    flow: Res<crate::system::app::policy::Flow>,
) -> Result<Progress, Fail> {
    if flow.settling || startup.at == startup.list.len() {
        return Ok(Progress::Done);
    }
    if let Some(result) = operations.completed_local_action() {
        let completed = result.map_err(|error| { crate::debug::put(&alloc::format!("system: startup failed {error:?}")); Fail::Shutdown })?;
        let action = match completed {
            Action::Mint => Action::Embark { parent: None },
            Action::Embark { .. } => {
                startup.at += 1;
                Action::Mint
            }
            _ => return Ok(Progress::Done),
        };
        if let Some(program) = startup.list.get(startup.at) {
            operations
                .submit(program.name().into(), action)
                .map_err(|_| Fail::Room)?;
        }
    } else if operations.is_empty() {
        operations
            .submit(startup.list[startup.at].name().into(), Action::Mint)
            .map_err(|_| Fail::Room)?;
    }
    Ok(Progress::Done)
}

pub fn eligibility(control: Res<Control>, mut startup: ResMut<Startup>) -> Result<Progress, Fail> {
    startup.eligible = startup.at == startup.list.len() && control.due();
    Ok(Progress::Done)
}
