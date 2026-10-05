use super::{
    Fail,
    answer::{Request as IdentityRequest, answer},
    face::Current,
};
use crate::system::{identity::core::IdentityBook, life::Status};
use alloc::sync::Arc;
use runtime::schedule::{Progress, Res, ResMut};
pub(super) struct Book(pub Option<IdentityBook>);
pub(super) fn initialize(
    status: Res<Arc<Status>>,
    mut book: ResMut<Book>,
) -> Result<Progress, Fail> {
    book.0 = Some(
        IdentityBook::new(env::unit::self_id(), status.control).map_err(|_| Fail::Book)?,
    );
    Ok(Progress::Done)
}
pub(super) fn apply(
    mut book: ResMut<Book>,
    mut current: ResMut<Current>,
) -> Result<Progress, Fail> {
    let Current::Received(incoming) = &mut *current else { return Err(Fail::Book); };
    let reply = answer(
        book.0.as_mut().ok_or(Fail::Book)?,
        IdentityRequest {
            from: incoming.request.from,
            grant: incoming.request.grant,
            wire: incoming.request.wire.take(),
        },
    );
    let Current::Received(incoming) = core::mem::replace(&mut *current, Current::Empty)
        else { unreachable!() };
    *current = Current::Answered { incoming, reply };
    Ok(Progress::Done)
}
