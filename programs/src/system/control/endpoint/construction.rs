//! Capability-granted creation admission; Identity validates the allowed subject subtree.
use ::schedule::{Progress, Res, ResMut};
use alloc::vec::Vec;
use env::{PieToken, TaskId, Wait};
use ipc::rpc;
use system_api::{
    control::{Fail, construction as api},
    loader::Said,
};
pub(crate) struct Incoming {
    request: api::Request,
    from: TaskId,
    back: rpc::reply::Sender<Said>,
}
pub(crate) struct Construction {
    pub entry: PieToken,
    pub creators: Vec<TaskId>,
    queued: Vec<(Incoming, bool)>,
}
impl Construction {
    pub(crate) fn new() -> Result<Self, &'static str> {
        Ok(Self {
            entry: env::pie::unseal(env::UnsealArgs::hole(api::ENTRY)).map_err(|_| "construction entry")?,
            creators: Vec::new(),
            queued: Vec::new(),
        })
    }
}
pub(crate) fn receive(
    mut inbox: ResMut<Construction>,
) -> Result<Progress, crate::system::app::Fault> {
    use wire::Message;
    let mut bytes = api::Request::EMPTY;
    let receiver = rpc::request::Receiver::<api::Call>::from_raw(
        inbox.entry,
        api::Call::BACK,
        api::Call::back,
    );
    for _ in 0..16 {
        let incoming = match receiver.receive(&mut bytes, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rejected) if matches!(rejected.fail, rpc::Fail::Receive(_)) => break,
            Err(rejected) => {
                if let Some((from, request)) = rejected.incoming {
                    crate::system::loader::release_image(&request.image, from);
                }
                continue;
            }
        };
        if inbox.queued.len() >= 16 || inbox.queued.try_reserve(1).is_err() {
            crate::system::loader::release_image(&incoming.request.image, incoming.from);
            let _ = incoming
                .reply
                .send(system_api::loader::Said::from_result(Err(Fail::Full)));
        } else {
            inbox.queued.push((
                Incoming {
                    request: incoming.request,
                    from: incoming.from,
                    back: incoming.reply,
                },
                false,
            ));
        }
    }
    Ok(Progress::Done)
}
pub(crate) fn admit(
    mut inbox: ResMut<Construction>,
    control: Res<crate::system::control::unit::Control>,
    roster: Res<crate::system::control::identity::Roster>,
) -> Result<Progress, crate::system::app::Fault> {
    inbox.creators.retain(|task| control.live(*task));
    let mut index = 0;
    while index < inbox.queued.len() {
        let item = &inbox.queued[index].0;
        let allowed = inbox.creators.contains(&item.from)
            && control.live(item.from)
            && control.live(item.request.owner)
            && roster
                .allow_subject(item.from, item.request.subject)
                .is_ok();
        let failure = if !allowed {
            Some(Fail::Denied)
        } else if control.owns_team_instance(item.request.owner) {
            Some(Fail::NotReady)
        } else {
            None
        };
        if let Some(fail) = failure {
            let (item, _) = inbox.queued.remove(index);
            crate::system::loader::release_image(&item.request.image, item.from);
            let _ = item
                .back
                .send(system_api::loader::Said::from_result(Err(fail)));
        } else {
            inbox.queued[index].1 = true;
            index += 1;
        }
    }
    Ok(Progress::Done)
}

impl Construction {
    pub(crate) fn grant(&mut self, task: TaskId) -> Result<(), &'static str> {
        if self.creators.contains(&task) {
            return Ok(());
        }
        self.creators
            .try_reserve(1)
            .map_err(|_| "constructor grant capacity")?;
        resource::port::ship(self.entry, task, env::Access::STORE, env::Policy::NONE)
            .map_err(|_| "constructor grant")?;
        self.creators.push(task);
        Ok(())
    }
    pub(crate) fn drain(
        &mut self,
    ) -> impl Iterator<Item = (api::Request, TaskId, rpc::reply::Sender<Said>, bool)> + '_ {
        self.queued
            .drain(..)
            .map(|(item, approved)| (item.request, item.from, item.back, approved))
    }
}
