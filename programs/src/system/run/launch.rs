use ::schedule::{Progress, ResMut};
use crate::system::{
    control::serve::unit::Control,
    loader::{Image, serve::build::Spawn},
};
use alloc::vec::Vec;
use env::TaskId;
use ipc::rpc::reply::Sender;
use protocol::{
    system::{
        control::Fail,
        identity::Install,
        loader::{Built, frame::Said},
    },
};
#[derive(Default)]
pub struct Pending(pub Vec<Launch>);
pub struct Launch {
    pub task: TaskId,
    pub identity: Install,
    back: Sender<Said>,
}
pub struct Build<'a> {
    pub image: Image<'a>,
    pub spawn: Spawn<'a>,
    pub delivery: Delivery,
}
pub struct Delivery {
    pub owner: TaskId,
    pub identity: Install,
    pub back: Sender<Said>,
}
pub fn construct(
    control: &mut Control,
    pending: &mut Pending,
    build: Build<'_>,
) -> Result<Built, (Fail, Sender<Said>)> {
    let Build { image, spawn, delivery } = build;
    if pending.0.try_reserve(1).is_err() {
        return Err((Fail::Full, delivery.back));
    }
    if let Err(fail) = control.reserve_instance() {
        return Err((fail, delivery.back));
    }
    let built = match crate::system::loader::serve::build::construct(
        &mut control.loader,
        image,
        spawn,
    ) {
        Ok(built) => built,
        Err(fail) => return Err((fail.into(), delivery.back)),
    };
    control.register_instance(built, delivery.owner);
    pending.0.push(Launch {
        task: built.task,
        identity: delivery.identity,
        back: delivery.back,
    });
    Ok(built)
}
pub(super) fn reply(back: Sender<Said>, result: Result<Built, Fail>) -> bool {
    let value = match result {
        Ok(built) => Said {
            status: protocol::wire::OK,
            task: built.task,
            team: built.team.get() as u64,
        },
        Err(fail) => Said {
            status: protocol::system::control::frame::fail_to_code(Some(fail)),
            task: TaskId::new(0),
            team: 0,
        },
    };
    back.send(value).is_ok()
}
pub fn completed(
    mut pending: ResMut<Pending>,
    mut control: ResMut<Control>,
) -> Result<Progress, crate::system::control::serve::Fail> {
    use crate::system::control::core::unit::State;
    let mut index = 0;
    while index < pending.0.len() {
        let task = pending.0[index].task;
        let item = control.instances.iter().find(|item| item.task == task);
        let result = match item {
            Some(item) if item.state == State::Debarked => Ok(Built {
                task: item.task,
                team: item.team.ok_or(crate::system::control::serve::Fail::Room)?,
            }),
            Some(item) if item.state == State::Starting => {
                index += 1;
                continue;
            }
            _ => Err(Fail::NotReady),
        };
        let launch = pending.0.remove(index);
        if !reply(launch.back, result) {
            control.stop_instance(launch.task);
        }
    }
    Ok(Progress::Done)
}
