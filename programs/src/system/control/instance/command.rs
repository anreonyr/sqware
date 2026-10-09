use crate::system::control::{unit::Control, unit::table::State};
use env::{TaskId, unit};
use system_api::control::Fail;

#[derive(Clone, Copy)]
pub(crate) enum Command {
    Embark(TaskId),
    Debark(TaskId),
    Ruin(TaskId),
    State(TaskId),
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
            | Command::State(task) => task,
        };
        let item = self
            .instances
            .iter_mut()
            .find(|item| item.task == task)
            .ok_or(Fail::Unknown)?;
        if item.owner != owner {
            return Err(Fail::Denied);
        }
        if item.team.is_some()
            && !item.claimed
            && matches!(command, Command::Embark(_))
            && env::chrono::clock() >= item.claim_until
        {
            item.stop();
            return Err(Fail::NotReady);
        }
        match command {
            Command::State(_) => return Ok(Some(item.state)),
            Command::Embark(_) if item.state == State::Debarked => {
                let running = if item.started { unit::embark_team(item.team.ok_or(Fail::Unknown)?) } else { unit::embark(task) };
                match running {
                    Ok(()) => {},
                    Err(error) if error.source == env::UnitFail::Busy => return Ok(None),
                    Err(_) => { item.stop(); return Err(Fail::NotReady); }
                }
                item.claimed = true;
                item.started = true;
                item.state = State::Ready;
            }
            Command::Embark(_) if item.state == State::Ready => {}
            Command::Debark(_) if item.state == State::Ready => {
                match unit::debark_team(item.team.ok_or(Fail::Unknown)?) {
                    Ok(()) => {}
                    Err(error) if error.source == env::UnitFail::Busy => return Ok(None),
                    Err(_) => return Err(Fail::NotReady),
                }
                item.state = State::Debarked;
            }
            Command::Ruin(_) => {
                if item.team.is_some() {
                    item.stop();
                    let _ = env::room::doom(task);
                    return Ok(None);
                }
            }
            _ => return Err(Fail::NotReady),
        }
        Ok(Some(item.state))
    }
}
