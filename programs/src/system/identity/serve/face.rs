use super::{Fail, answer::Request as IdentityRequest};
use crate::system::{common::face::mount, life::Status};
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use env::{HoleDir, PieToken, Wait};
use protocol::{
    common::schedule::{Dispatch, Invocation, Progress, Res, ResMut},
    communication::hand::Sender,
    system::identity::{self as api, Grant, Reply, Wire},
};
use runtime::{
    core::res::{
        pile::Pile,
        port::{self, Access, Policy},
    },
    env::mail::{self, HolePie},
};
pub(super) struct Faces(pub Vec<(PieToken, Grant)>);
pub(super) struct Buffer(pub Vec<u8>);
pub(super) struct Inbox(pub VecDeque<Incoming>);
pub(super) struct Incoming {
    pub request: IdentityRequest,
    pub back: PieToken,
}
pub(super) struct Current(pub Option<Incoming>);
pub(super) struct Response(pub Option<Reply>);
pub(super) struct Ready(pub Option<(PieToken, Grant)>);
pub(super) fn faces(
    status: Res<Arc<Status>>,
    mut faces: ResMut<Faces>,
    pile: Res<Pile>,
) -> Result<Progress, Fail> {
    faces
        .0
        .try_reserve_exact(Grant::ALL.len())
        .map_err(|_| Fail::Room)?;
    for grant in Grant::ALL {
        let (token, _) = mount::entry(grant.mark(), grant.name()).map_err(|_| Fail::Tree)?;
        port::ship(
            &HolePie::from_token(token),
            status.control,
            Access::FETCH | Access::STORE,
            Policy::VEST,
        )
        .map_err(|_| Fail::Tree)?;
        pile.attach(&HolePie::from_token(token), HoleDir::Pull)
            .map_err(|_| Fail::Desk)?;
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
        .and_then(|(entry, _)| faces.0.iter().find(|(token, _)| *token == entry).copied());
    Ok(Progress::Done)
}
pub(super) fn receive(
    ready: Res<Ready>,
    mut buffer: ResMut<Buffer>,
    mut inbox: ResMut<Inbox>,
) -> Result<Progress, Fail> {
    if let Some((entry, grant)) = &ready.0 {
        while let Ok((n, from)) = HolePie::from_token(*entry).pull(&mut buffer.0, Wait::POLL) {
            let Some((wire, back)) = Wire::take(&buffer.0[..n]) else {
                continue;
            };
            if !matches!(mail::reserve(back), Ok((_, owner, mark)) if owner == from && mark == api::BACK)
            {
                continue;
            }
            inbox.0.try_reserve(1).map_err(|_| Fail::Room)?;
            inbox.0.push_back(Incoming {
                request: IdentityRequest {
                    from,
                    grant: *grant,
                    wire,
                },
                back,
            });
        }
    }
    Ok(Progress::Done)
}
pub(super) fn budget(
    inbox: Res<Inbox>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    dispatch.budget = inbox.0.len();
    Ok(Progress::Done)
}
pub(super) fn select(
    mut inbox: ResMut<Inbox>,
    mut current: ResMut<Current>,
    mut dispatch: ResMut<Dispatch<(), Fail>>,
) -> Result<Progress, Fail> {
    current.0 = inbox.0.pop_front();
    if current.0.is_some() {
        dispatch.current = Some(Invocation {
            key: (),
            cursor: Default::default(),
        });
    }
    Ok(Progress::Done)
}
pub(super) fn reply(
    mut current: ResMut<Current>,
    mut response: ResMut<Response>,
) -> Result<Progress, Fail> {
    let incoming = current.0.take().ok_or(Fail::Book)?;
    if let Some(response) = response.0.take() {
        let _ = Sender::<Reply>::from_token(incoming.back).send(response);
    }
    let _ = mail::release(incoming.back);
    Ok(Progress::Done)
}
pub(super) fn finish(mut dispatch: ResMut<Dispatch<(), Fail>>) -> Result<Progress, Fail> {
    if dispatch.result.take().is_some_and(|result| result.is_err()) {
        return Err(Fail::Book);
    }
    dispatch.current = None;
    Ok(Progress::Done)
}
pub(super) fn close(faces: Res<Faces>, pile: Res<Pile>) -> Result<Progress, Fail> {
    for (entry, _) in &faces.0 {
        let _ = pile.detach(&HolePie::from_token(*entry), HoleDir::Pull);
        let _ = mail::seal(*entry);
        let _ = mail::release(*entry);
    }
    Ok(Progress::Done)
}
