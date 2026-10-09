use crate::system::{
    control::unit::Control,
    loader::{Image, Spawn},
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
    pub built: Built,
    pub delivery: Delivery,
    registered: bool,
}
pub struct Delivery {
    pub owner: TaskId,
    pub identity: Install,
    pub back: Sender<Said>,
}
pub struct Request {
    pub ask: system_api::loader::Ask,
    pub from: TaskId,
    pub delivery: Delivery,
}
#[derive(Default)]
pub struct Requests(pub Vec<Request>);

pub(crate) fn register(mut pending: ResMut<Pending>, mut control: ResMut<Control>) -> Result<Progress, crate::system::app::Fault> {
    let mut index = 0;
    while index < pending.0.len() {
        if !pending.0[index].registered {
            let launch = &pending.0[index];
            if let Err(fail) = control.create_instance(launch.built, launch.delivery.owner) {
                let launch = pending.0.remove(index);
                reply(launch.delivery.back, Err(fail));
                continue;
            }
            pending.0[index].registered = true;
        }
        index += 1;
    }
    Ok(Progress::Done)
}
pub(crate) fn construct(loader: &mut crate::system::loader::Loader, pending: &mut Pending, request: Request) {
    let Request { ask, from, delivery } = request;
    let result = (|| {
        let bytes = crate::system::loader::snapshot(crate::system::loader::Source { from, ask: &ask })?;
        let count = ask.count as usize;
        if count > system_api::loader::MAX_ARGS { return Err(system_api::loader::Fail::Bad); }
        pending.0.try_reserve(1).map_err(|_| system_api::loader::Fail::Full)?;
        let mut args = [0; system_api::loader::MAX_ARGS];
        for (to, from) in args.iter_mut().zip(&ask.args[..count]) { *to = *from as usize; }
        loader.construct(Image { bytes: &bytes, kind: env::ProgramKind::User }, Spawn { args: &args[..count], stack: ask.stack as usize })
    })();
    crate::system::loader::release_image(&ask, from);
    match result {
        Ok(built) => pending.0.push(Launch { built, delivery, registered: false }),
        Err(fail) => { reply(delivery.back, Err(fail.into())); }
    }
}
pub(super) fn reply(back: Sender<Said>, result: Result<Built, Fail>) -> bool {
    let value = Said::from_result(result);
    back.send(value).is_ok()
}
pub fn completed(
    mut pending: ResMut<Pending>,
    mut control: ResMut<Control>,
) -> Result<Progress, crate::system::app::Fault> {
    let mut index = 0;
    while index < pending.0.len() {
        if !pending.0[index].registered {
            index += 1;
            continue;
        }
        let task = pending.0[index].built.task;
        let result = match control.instance_result(task)? {
            Some(result) => result,
            None => {
                index += 1;
                continue;
            }
        };
        let launch = pending.0.remove(index);
        if !reply(launch.delivery.back, result) {
            control.stop_instance(launch.built.task);
        }
    }
    Ok(Progress::Done)
}

pub(crate) mod hooks;

pub(crate) fn install(resources: &mut ::schedule::Resources<'static>) -> Result<(), ::schedule::resource::AccessError> {
    resources
        .insert(Requests::default())?
        .insert(Pending::default())?;
    Ok(())
}

pub(crate) fn dispatch(mut inbox: ResMut<crate::system::control::Construction>, mut requests: ResMut<crate::system::launch::Requests>) -> Result<Progress, crate::system::app::Fault> {
    for (request, from, back, approved) in inbox.drain() {
        if !approved || requests.0.len() >= 16 || requests.0.try_reserve(1).is_err() {
            crate::system::loader::release_image(&request.image, from);
            crate::system::launch::reply(back, Err(system_api::control::Fail::Full));
            continue;
        }
        requests.0.push(crate::system::launch::Request { ask: request.image, from: from,
            delivery: crate::system::launch::Delivery { owner: request.owner, identity: system_api::identity::Install::Authorized(request.subject), back: back } });
    }
    Ok(Progress::Done)
}
