use crate::system::control::{unit::Control, unit::table::State};
use env::{TaskId, unit};
use system_api::control::Fail;

#[derive(Clone, Copy)]
pub(crate) enum Command {
    Embark(TaskId),
    Debark(TaskId),
    Ruin(TaskId),
    State(TaskId),
    Release(TaskId),
}

impl Control {
    pub(crate) fn command_instance(
        &mut self,
        owner: TaskId,
        command: Command,
    ) -> Result<Option<State>, Fail> {
        let task = match command {
            Command::Embark(task)
            | Command::Debark(task)
            | Command::Ruin(task)
            | Command::State(task)
            | Command::Release(task) => task,
        };
        if matches!(command, Command::Release(_)) {
            return self.release_instance(owner, task);
        }
        let item = self
            .instances
            .iter_mut()
            .find(|item| item.task == task)
            .ok_or(Fail::Unknown)?;
        if item.owner != owner {
            return Err(Fail::Denied);
        }
        match command {
            Command::State(_) => return Ok(Some(item.state)),
            Command::Embark(_) if item.state == State::Starting && item.started => {
                return Ok(Some(State::Starting));
            }
            Command::Embark(_) if item.state == State::Debarked => {
                if item.claim(env::chrono::clock()).is_none() {
                    item.stop();
                    if let Some(team) = item.team() {
                        let _ = unit::slay_team(team);
                    }
                    return Err(Fail::NotReady);
                }
                let resumed = item.started;
                let running = if resumed {
                    unit::embark_team(item.team().ok_or(Fail::Unknown)?)
                } else {
                    unit::embark_task(task)
                };
                match running {
                    Ok(()) => {}
                    Err(error) if error.source == env::UnitFail::Busy => return Ok(None),
                    Err(_) => {
                        item.stop();
                        if let Some(team) = item.team() {
                            let _ = unit::slay_team(team);
                        }
                        return Err(Fail::NotReady);
                    }
                }
                item.started = true;
                // A resumed team already passed readiness before it was debarked.
                item.state = if resumed {
                    State::Ready
                } else {
                    State::Starting
                };
            }
            Command::Embark(_) if item.state == State::Ready => {}
            Command::Debark(_) if item.state == State::Ready => {
                match unit::debark_team(item.team().ok_or(Fail::Unknown)?) {
                    Ok(()) => {}
                    Err(error) if error.source == env::UnitFail::Busy => return Ok(None),
                    Err(_) => return Err(Fail::NotReady),
                }
                item.state = State::Debarked;
            }
            Command::Ruin(_) => {
                if let Some(team) = item.team() {
                    item.stop();
                    let _ = unit::slay_team(team);
                    return Ok(None);
                }
            }
            _ => return Err(Fail::NotReady),
        }
        Ok(Some(item.state))
    }
}
