use crate::system::{
    control::unit::Control,
    loader::{Image, Spawn},
};
use ::schedule::{BuildError, Plan, Progress, Res, ResMut, Schedule};
use alloc::vec::Vec;
use env::TaskId;
use ipc::rpc::reply::Sender;
use system_api::control::Fail;
use system_api::identity::Install;
use system_api::loader::Built;
use system_api::loader::Said;
#[derive(Default)]
pub struct Pending {
    requests: Vec<Request>,
    launches: Vec<Launch>,
}
pub struct Launch {
    pub task: TaskId,
    pub delivery: Delivery,
}
pub struct Delivery {
    pub owner: TaskId,
    pub identity: Install,
    pub constructor: bool,
    pub back: Sender<Said>,
}
pub struct Request {
    pub ask: system_api::loader::Ask,
    pub from: TaskId,
    pub delivery: Delivery,
}
impl Pending {
    pub(crate) fn push(&mut self, request: Request) {
        if self.requests.len() >= 16 || self.requests.try_reserve(1).is_err() {
            crate::system::loader::release_image(&request.ask, request.from);
            reply(request.delivery.back, Err(Fail::Full));
        } else {
            self.requests.push(request);
        }
    }
}
pub(crate) fn frame() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    let mut frame = Schedule::sequence();
    frame.system("dispatch", dispatch)?;
    frame.plan("receive", crate::system::loader::frame()?)?;
    frame.system("settle", settle)?;
    frame.system("construct", build)?;
    frame.build()
}
fn settle(
    flow: Res<crate::system::app::policy::Flow>,
    mut pending: ResMut<Pending>,
) -> Result<Progress, crate::system::app::Fault> {
    if flow.settling {
        for request in pending.requests.drain(..) {
            crate::system::loader::release_image(&request.ask, request.from);
            reply(request.delivery.back, Err(Fail::NotReady));
        }
    }
    Ok(Progress::Done)
}
fn build(
    mut loader: ResMut<crate::system::loader::Loader>,
    mut control: ResMut<Control>,
    mut pending: ResMut<Pending>,
) -> Result<Progress, crate::system::app::Fault> {
    let pending = &mut *pending;
    for request in pending.requests.drain(..) {
        if pending.launches.try_reserve(1).is_err() {
            crate::system::loader::release_image(&request.ask, request.from);
            reply(request.delivery.back, Err(Fail::Full));
            continue;
        }
        if let Some(launch) = construct(&mut loader, &mut control, request) {
            pending.launches.push(launch);
        }
    }
    Ok(Progress::Done)
}
fn construct(
    loader: &mut crate::system::loader::Loader,
    control: &mut Control,
    request: Request,
) -> Option<Launch> {
    let Request { ask, from, delivery } = request;
    let result = control.create_instance(delivery.owner, || {
        let count = ask.count as usize;
        if count > system_api::loader::MAX_ARGS { return Err(system_api::loader::Fail::Bad.into()); }
        let bytes = crate::system::loader::snapshot(crate::system::loader::Source { from, ask: &ask })?;
        let mut args = [0; system_api::loader::MAX_ARGS];
        for (to, from) in args.iter_mut().zip(&ask.args[..count]) { *to = *from as usize; }
        loader.construct(Image { bytes: &bytes, kind: env::ProgramKind::User }, Spawn { args: &args[..count], stack: ask.stack as usize }).map_err(Into::into)
    });
    crate::system::loader::release_image(&ask, from);
    match result {
        Ok(built) => Some(Launch { task: built.task, delivery }),
        Err(fail) => { reply(delivery.back, Err(fail)); None }
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
    while index < pending.launches.len() {
        let task = pending.launches[index].task;
        let result = match control.instance_result(task)? {
            Some(result) => result,
            None => {
                index += 1;
                continue;
            }
        };
        let launch = pending.launches.remove(index);
        if !reply(launch.delivery.back, result) {
            control.stop_instance(launch.task);
        }
    }
    Ok(Progress::Done)
}

pub(crate) mod hooks;

pub(crate) fn install(resources: &mut ::schedule::Resources<'static>) -> Result<(), &'static str> {
    resources
        .insert(Pending::default())
        .map_err(|_| "launch resource capacity")?;
    Ok(())
}

fn dispatch(mut inbox: ResMut<crate::system::control::Construction>, mut pending: ResMut<Pending>) -> Result<Progress, crate::system::app::Fault> {
    for (request, from, back, approved) in inbox.drain() {
        if !approved {
            crate::system::loader::release_image(&request.image, from);
            reply(back, Err(Fail::Denied));
            continue;
        }
        pending.push(Request { ask: request.image, from: from,
            delivery: Delivery { owner: request.owner, identity: system_api::identity::Install::Authorized(request.subject), constructor: request.constructor, back: back } });
    }
    Ok(Progress::Done)
}
