use super::{
    Fail,
    book::Book,
    face::{Buffer, Current, Faces, Inbox, Ready},
    frame::Running,
};
use crate::system::life::Status;
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use runtime::schedule::{Cursor, Dispatch, Progress, Resources};
use runtime::core::res::pile::Pile;
pub fn serve(
    status: Arc<Status>,
    epoch: crate::system::identity::revision::Epoch,
    changed: crate::system::identity::revision::Changed,
) -> Result<(), Fail> {
    let mut resources = Resources::new();
    resources.insert(epoch).map_err(|_| Fail::Room)?;
    resources.insert(changed).map_err(|_| Fail::Room)?;
    resources.insert(status).map_err(|_| Fail::Room)?;
    resources.insert(Book(None)).map_err(|_| Fail::Room)?;
    resources
        .insert(Faces(Vec::new()))
        .map_err(|_| Fail::Room)?;
    resources
        .insert(Pile::unseal(false).map_err(|_| Fail::Desk)?)
        .map_err(|_| Fail::Room)?;
    resources
        .insert(Buffer(alloc::vec![0; runtime::PAGE_SIZE]))
        .map_err(|_| Fail::Room)?;
    resources
        .insert(Inbox(VecDeque::new()))
        .map_err(|_| Fail::Room)?;
    resources.insert(Current::Empty).map_err(|_| Fail::Room)?;
    resources.insert(Ready(None)).map_err(|_| Fail::Room)?;
    resources.insert(Running(true)).map_err(|_| Fail::Room)?;
    resources
        .insert(Dispatch::<(), Fail>::new())
        .map_err(|_| Fail::Room)?;
    let [mut start, mut frame, mut stop] = super::schedule::plans().map_err(|_| Fail::Room)?;
    start.prepare(&resources);
    frame.prepare(&resources);
    stop.prepare(&resources);
    start
        .advance(&mut Cursor::default(), &resources)
        .map_err(|_| Fail::Book)?;
    let mut cursor = Cursor::default();
    while resources.read::<Running>().map_err(|_| Fail::Room)?.0 {
        if frame
            .advance(&mut cursor, &resources)
            .map_err(|_| Fail::Dead)?
            == Progress::Done
        {
            cursor.reset();
        }
    }
    stop.advance(&mut Cursor::default(), &resources)
        .map_err(|_| Fail::Desk)?;
    Ok(())
}
