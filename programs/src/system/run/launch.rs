use crate::system::{
    control::unit::Control,
    loader::{Image, serve::build::Spawn},
};
use ::schedule::{Progress, ResMut};
use alloc::vec::Vec;
use env::TaskId;
use ipc::rpc::reply::Sender;
use system_api::control::Fail;
use system_api::identity::Install;
use system_api::loader::Built;
use system_api::loader::Said;
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
    let Build {
        image,
        spawn,
        delivery,
    } = build;
    if pending.0.try_reserve(1).is_err() {
        return Err((Fail::Full, delivery.back));
    }
    let built = match control.create_instance(crate::system::control::instance::create::Creation {
        image,
        spawn,
        owner: delivery.owner,
    }) {
        Ok(built) => built,
        Err(fail) => return Err((fail.into(), delivery.back)),
    };
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
            status: wire::OK,
            task: built.task,
            team: built.team.get() as u64,
        },
        Err(fail) => Said {
            status: system_api::control::frame::fail_to_code(Some(fail)),
            task: TaskId::new(0),
            team: 0,
        },
    };
    back.send(value).is_ok()
}
pub fn completed(
    mut pending: ResMut<Pending>,
    mut control: ResMut<Control>,
) -> Result<Progress, crate::system::control::Fail> {
    let mut index = 0;
    while index < pending.0.len() {
        let task = pending.0[index].task;
        let result = match control.instance_result(task)? {
            Some(result) => result,
            None => {
                index += 1;
                continue;
            }
        };
        let launch = pending.0.remove(index);
        if !reply(launch.back, result) {
            control.stop_instance(launch.task);
        }
    }
    Ok(Progress::Done)
}
