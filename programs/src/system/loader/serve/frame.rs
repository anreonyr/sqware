use super::{
    answer::{self, Inbox},
    build::construct,
};
use crate::system::control::core::unit::State;
use crate::system::{
    control::serve::{Fail, unit::Control},
    identity::serve::install::Roster,
};
use env::TaskId;
use protocol::{
    common::schedule::{Progress, Res, ResMut},
    system::loader::frame::{self, Said},
};

pub fn build(
    mut control: ResMut<Control>,
    roster: Res<Roster>,
    mut inbox: ResMut<Inbox>,
    flow: Res<crate::system::control::serve::frame::Flow>,
) -> Result<Progress, Fail> {
    for incoming in inbox.requests.drain(..) {
        let result = if flow.settling {
            Err(frame::Fail::NotReady)
        } else {
            construct(&mut control, &roster, &incoming)
        };
        answer::release_image(&incoming.ask, incoming.from);
        let said = match result {
            Ok(built) => Said {
                status: protocol::wire::OK,
                team: built.team.get() as u64,
                task: built.task,
            },
            Err(fail) => Said {
                status: protocol::system::control::frame::fail_to_code(Some(fail)),
                team: 0,
                task: TaskId::new(0),
            },
        };
        if !answer::reply(incoming.ask.back, said) && said.task.get() != 0 {
            if let Some(item) = control
                .instances
                .iter_mut()
                .find(|item| item.task == said.task)
            {
                item.state = State::Stopping;
            }
        }
    }
    Ok(Progress::Done)
}
