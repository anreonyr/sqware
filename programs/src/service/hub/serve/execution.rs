use env::{TaskId, TeamId, Wait};
use resource::raw::Hole;
use system_api::control::{Fail, State};

use crate::service::hub::core::{
    ExecutionEffects, ExecutionFailure as Failure, ExecutionStep, Ledger,
};

use super::peers::Peers;

const MS: usize = 3000;

struct Effects<'a>(&'a Peers);

impl ExecutionEffects for Effects<'_> {
    fn team(&mut self, task: TaskId) -> Result<Option<TeamId>, Failure> {
        match self.0.control.instance(task).inspect(Wait::AtMost(MS)) {
            Ok(said) if said.team != TeamId::new(0) => Ok(Some(said.team)),
            Ok(said) if State::of_code(said.a) == Some(State::Dead) => Ok(None),
            Ok(_) => Err(Failure::Control(Fail::Bad)),
            Err(Fail::Unknown | Fail::Denied) => Ok(None),
            Err(fail) => Err(Failure::Control(fail)),
        }
    }

    fn embark(&mut self, task: TaskId) -> Result<(), Failure> {
        self.0.control.instance(task).embark(Wait::AtMost(MS))
            .map_err(Failure::Control)
    }

    fn exited(&mut self, task: TaskId) -> Result<bool, Failure> {
        env::unit::join_task(task, Wait::POLL)
            .map_err(|error| Failure::Unit(error.source))
    }

    fn stop(&mut self, team: TeamId) -> Result<(), Failure> {
        env::unit::slay_team(team).map_err(|error| Failure::Unit(error.source))
    }

    fn oust(&mut self, team: TeamId) -> Result<bool, Failure> {
        match env::unit::oust(team) {
            Ok(()) => Ok(true),
            Err(error) if error.source == env::UnitFail::Busy => Ok(false),
            Err(error) => Err(Failure::Unit(error.source)),
        }
    }

}

pub fn advance(ledger: &mut Ledger, peers: &Peers) {
    receive(ledger);
    let mut effects = Effects(peers);
    for execution in ledger.executions() {
        let before = execution.step;
        let first_failure = execution.failure.is_none();
        let result = execution.advance(env::chrono::clock(), &mut effects);
        if let Err(failure) = result {
            if first_failure {
                crate::debug::put(&alloc::format!(
                    "hub: execution {} step={before:?} failed {failure:?} (holding)",
                    execution.task.get(),
                ));
            }
        }
        if before == ExecutionStep::Embark && execution.step == ExecutionStep::Running {
            crate::debug::put(&alloc::format!(
                "hub: execution instance launched tid={}", execution.task.get(),
            ));
        }
    }
    ledger.prune_executions();
}

fn receive(ledger: &mut Ledger) {
    for info in resource::raw::pies().filter(|info| {
        info.alive && info.kind == env::PieKind::Hole && info.mark == hub_api::LAUNCH_MARK
    }) {
        let mut bytes = [0u8; 8];
        loop {
            // Reserve before consuming the only copy of a launch request.
            if ledger.reserve_execution().is_err() {
                return;
            }
            let Ok((len, _)) = Hole::from_raw(info.token).pull(&mut bytes, Wait::POLL) else {
                break;
            };
            if len != bytes.len() {
                continue;
            }
            let Ok(raw) = usize::try_from(u64::from_le_bytes(bytes)) else {
                continue;
            };
            if raw != 0 {
                let deadline = env::chrono::clock().saturating_add(MS as u64 * 1_000_000);
                let _ = ledger.launch(TaskId::new(raw), deadline);
            }
        }
    }
}
