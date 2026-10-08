use crate::system::run::publication::book::Publications;
use ::schedule::{Progress, Res, ResMut};
use env::Wait;
use ipc::rpc;
use system_api::control::publication as pubcall;
use system_api::control::publication::Call as Publication;
use system_api::control::publication::Frame;
use system_api::control::publication::Reply;
use system_api::operator::Fail;

use super::{Inbox, Incoming, Outcome, Request};
use ::resource::raw::inspect;
use env::pie;
pub fn receive(
    images: Res<crate::system::control::unit::start::Images>,
    mut inbox: ResMut<Inbox>,
    mut dispatch: ResMut<::schedule::Dispatch<u8, &'static str>>,
) -> Result<Progress, &'static str> {
    let mut bytes = [0; Frame::LEN];
    let receiver = rpc::request::Receiver::<Publication>::from_raw(
        images.entry,
        Publication::BACK,
        Publication::back,
    );
    loop {
        let (frame, from, back) = match receiver.receive(&mut bytes, Wait::POLL) {
            Ok(incoming) => (incoming.request, incoming.from, Some(incoming.reply)),
            Err(rejected) => {
                if matches!(rejected.fail, rpc::Fail::Receive(_)) {
                    break;
                }
                let Some((from, frame)) = rejected.incoming else {
                    continue;
                };
                if frame.op != pubcall::PUBLISH
                    || !matches!(inspect(frame.entry), Ok((vestor, owner, _)) if vestor == from && owner == from)
                {
                    continue;
                }
                // Cleanup consults the index before forgetting an existing reference.
                (frame, from, None)
            }
        };
        inbox.0.try_reserve(1).map_err(|_| "publication capacity")?;
        inbox.0.push_back(Incoming { frame, from, back });
    }
    dispatch
        .begin(inbox.0.len())
        .map_err(|_| "publication scheduling error")?;
    Ok(Progress::Done)
}
pub fn select(
    mut inbox: ResMut<Inbox>,
    mut request: ResMut<Request>,
    mut dispatch: ResMut<::schedule::Dispatch<u8, &'static str>>,
) -> Result<Progress, &'static str> {
    request.0 = inbox.0.pop_front();
    if let Some(incoming) = &request.0 {
        dispatch
            .select(::schedule::Invocation {
                key: 0,
                cursor: Default::default(),
            })
            .map_err(|_| "publication scheduling error")?;
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
    let Some(back) = incoming.back else {
        return Ok(Progress::Done);
    };
    let _ = back.send(
        outcome
            .0
            .take()
            .unwrap_or(Err(Fail::Denied))
            .unwrap_or_else(Reply::fail),
    );
    Ok(Progress::Done)
}
pub fn completed(
    mut dispatch: ResMut<::schedule::Dispatch<u8, &'static str>>,
) -> Result<Progress, &'static str> {
    match dispatch
        .take_result()
        .map_err(|_| "publication scheduling error")?
        .result
    {
        Ok(Progress::Done) => Ok(Progress::Done),
        Err(::schedule::RunError::Step(why)) => Err(why),
        _ => Err("publication scheduling error"),
    }
}
