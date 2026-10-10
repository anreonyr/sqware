use super::{Fail, answer::Request as IdentityRequest};
use crate::support::face::mount;
use crate::system::app::life::Status;
use ::resource::{
    pile::Pile,
    port::{self, Access, Policy},
};
use ::schedule::{Dispatch, Invocation, Progress, Res, ResMut};
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use env::pie;
use env::{MailCondition, PieToken, Wait};
use ipc::rpc;
use system_api::identity as api;
use system_api::identity::Call as Contract;
use system_api::identity::Grant;
use system_api::identity::Reply;
pub(super) struct Faces(pub Vec<(PieToken, Grant)>);
pub(super) struct Buffer(pub Vec<u8>);
pub(super) struct Inbox(pub VecDeque<Incoming>);
pub(super) struct Incoming {
    pub request: IdentityRequest,
    pub back: rpc::reply::Sender<Reply>,
}
pub(super) enum Current {
    Empty,
    Received(Incoming),
    Answered { incoming: Incoming, reply: Reply },
}
impl Current {
    pub(super) fn mutated(&self) -> bool {
        matches!(self, Self::Answered { incoming, reply }
            if incoming.request.grant.mount() != api::Mount::Public
                && !matches!(reply, Reply::Fail(_)))
    }
}
pub(super) struct Ready(pub Option<(PieToken, Grant)>);
pub(super) fn faces(
    status: Res<Arc<Status>>,
    mut faces: ResMut<Faces>,
    pile: Res<Pile>,
) -> Result<Progress, Fail> {
    faces
        .0
        .try_reserve_exact(Grant::ALL.len())?;
    for grant in Grant::ALL {
        let (token, _) = mount::entry(grant.mark(), grant.name()).map_err(|_| Fail::Tree)?;
        port::ship(
            token,
            status.control,
            Access::FETCH | Access::STORE,
            Policy::VEST,
        )
        .map_err(|_| Fail::Tree)?;
        pile.attach(env::Source::Mail { pie: token, condition: MailCondition::Pull }).map_err(|_| Fail::Desk)?;
        faces.0.push((token, grant));
    }
    Ok(Progress::Done)
}
pub(super) fn wait(
    pile: Res<Pile>,
    faces: Res<Faces>,
    mut ready: ResMut<Ready>,
) -> Result<Progress, Fail> {
    ready.0 = pile
        .await_(Wait::AtMost(100))
        .map_err(|_| Fail::Dead)?
        .mail().and_then(|(entry, _)| faces.0.iter().find(|(token, _)| *token == entry).copied());
    Ok(Progress::Done)
}
pub(super) fn receive(
    ready: Res<Ready>,
    mut buffer: ResMut<Buffer>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, Fail> {
    if let Some((entry, grant)) = &ready.0 {
        let receiver =
            rpc::request::Receiver::<Contract>::from_raw(*entry, Contract::BACK, Contract::back);
        loop {
            let incoming = match receiver.receive(&mut buffer.0, Wait::POLL) {
                Ok(incoming) => incoming,
                Err(rejected) if matches!(rejected.fail, rpc::Fail::Receive(_)) => break,
                Err(_) => continue,
            };
            let (wire, _) = incoming.request;
            inbox.0.try_reserve(1)?;
            inbox.0.push_back(Incoming {
                request: IdentityRequest {
                    from: incoming.from,
                    grant: *grant,
                    wire,
                },
                back: incoming.reply,
            });
        }
    }
    Ok(Progress::Done)
}
pub(super) fn budget(
    inbox: Res<Inbox>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    dispatch.begin(inbox.0.len())?;
    Ok(Progress::Done)
}
pub(super) fn select(
    mut inbox: ResMut<Inbox>,
    mut current: ResMut<Current>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    *current = inbox
        .0
        .pop_front()
        .map(Current::Received)
        .unwrap_or(Current::Empty);
    if matches!(*current, Current::Received(_)) {
        dispatch
            .select(Invocation {
                key: (),
                cursor: Default::default(),
            })?;
    }
    Ok(Progress::Done)
}
pub(super) fn reply(mut current: ResMut<Current>) -> Result<Progress, Fail> {
    if !matches!(*current, Current::Answered { .. }) {
        return Err(Fail::Book);
    }
    let Current::Answered { incoming, reply } = core::mem::replace(&mut *current, Current::Empty)
    else {
        unreachable!()
    };
    let _ = incoming.back.send(reply);
    Ok(Progress::Done)
}
pub(super) fn finish(mut dispatch: ResMut<Dispatch<(), Fail>>) -> Result<Progress, Fail> {
    if dispatch
        .take_result()?
        .result
        .is_err()
    {
        return Err(Fail::Book);
    }
    Ok(Progress::Done)
}
pub(super) fn close(faces: Res<Faces>, pile: Res<Pile>) -> Result<Progress, Fail> {
    for (entry, _) in &faces.0 {
        let _ = pile.detach(env::Source::Mail { pie: *entry, condition: MailCondition::Pull });
        let _ = pie::seal(*entry);
        let _ = pie::release(*entry, env::ReleaseMode::Revoke);
    }
    Ok(Progress::Done)
}
