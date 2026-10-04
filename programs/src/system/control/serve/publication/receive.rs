use crate::system::control::core::publication::Publications;
use env::wire::Span as _;
use env::{PieToken, TaskId, Wait};
use protocol::common::schedule::{Progress, Res, ResMut};
use protocol::system::control::publication::{self as pubcall, Frame, Reply};
use protocol::system::operator::Fail;

use super::{Inbox, Incoming, Outcome, Request};
use env::pie;
use runtime::core::res::pie::{HolePie, inspect, reserve};
fn valid_back(back: PieToken, from: TaskId) -> bool {
    matches!(reserve(back), Ok((vestor, owner, mark)) if vestor == from && owner == from && mark == pubcall::BACK)
}
fn reply(back: PieToken, reply: Reply) {
    let mut bytes = [0; Reply::LEN];
    if let Some(n) = reply.store_at(&mut bytes, 0) {
        let _ = HolePie::from_token(back).push(&bytes[..n], Wait::POLL);
    }
    let _ = pie::release(back);
}
pub(crate) fn inject(entry: env::PieToken, task: env::TaskId) -> Result<(), &'static str> {
    runtime::core::res::port::ship(
        &HolePie::from_token(entry),
        task,
        env::Access::STORE,
        env::Policy::NONE,
    )
    .map(|_| ())
    .map_err(|_| "publication inject")
}
pub fn receive(
    images: Res<super::super::start::Images>,
    mut inbox: ResMut<Inbox>,
    mut dispatch: ResMut<protocol::common::schedule::Dispatch<u8, &'static str>>,
) -> Result<Progress, &'static str> {
    let mut bytes = [0; Frame::LEN];
    while let Ok((n, from)) = HolePie::from_token(images.entry).pull(&mut bytes, Wait::POLL) {
        let Some(frame) = Frame::take(&bytes[..n]) else {
            continue;
        };
        if !valid_back(frame.back, from) {
            if frame.op == pubcall::PUBLISH
                && matches!(inspect(frame.entry), Ok((vestor, owner, _)) if vestor == from && owner == from)
            {
                // Existing publications may still own an equivalent reference; cleanup runs with the index.
                inbox.0.try_reserve(1).map_err(|_| "publication capacity")?;
                inbox.0.push_back(Incoming {
                    frame,
                    from,
                    admitted: false,
                });
            }
            continue;
        }
        inbox.0.try_reserve(1).map_err(|_| "publication capacity")?;
        inbox.0.push_back(Incoming {
            frame,
            from,
            admitted: true,
        });
    }
    dispatch.budget = inbox.0.len();
    Ok(Progress::Done)
}
pub fn select(
    mut inbox: ResMut<Inbox>,
    mut request: ResMut<Request>,
    mut dispatch: ResMut<protocol::common::schedule::Dispatch<u8, &'static str>>,
) -> Result<Progress, &'static str> {
    request.0 = inbox.0.pop_front();
    if let Some(incoming) = &request.0 {
        dispatch.current = Some(protocol::common::schedule::Invocation {
            key: 0,
            cursor: Default::default(),
        });
        let _ = incoming;
    }
    Ok(Progress::Done)
}
pub fn finish(
    mut request: ResMut<Request>,
    mut outcome: ResMut<Outcome>,
    publications: Res<Publications>,
) -> Result<Progress, &'static str> {
    let incoming = request.0.take().ok_or("publication request")?;
    if incoming.frame.op == pubcall::PUBLISH
        && matches!(inspect(incoming.frame.entry), Ok((vestor, owner, _)) if vestor == incoming.from && owner == incoming.from)
        && !publications.owns(incoming.frame.entry)
    {
        let _ = pie::forget(incoming.frame.entry);
    }
    if !incoming.admitted {
        return Ok(Progress::Done);
    }
    reply(
        incoming.frame.back,
        outcome
            .0
            .take()
            .unwrap_or(Err(Fail::Denied))
            .unwrap_or_else(Reply::fail),
    );
    Ok(Progress::Done)
}
pub fn completed(
    mut dispatch: ResMut<protocol::common::schedule::Dispatch<u8, &'static str>>,
) -> Result<Progress, &'static str> {
    dispatch.current = None;
    match dispatch.result.take() {
        Some(Ok(Progress::Done)) => Ok(Progress::Done),
        Some(Err(protocol::common::schedule::RunError::Step(why))) => Err(why),
        _ => Err("publication scheduling error"),
    }
}
