use super::{
    Fail,
    book::Book,
    face::{Buffer, Current, Faces, Inbox, Ready},
    frame::Running,
};
use crate::system::app::life::Status;
use ::resource::pile::Pile;
use ::schedule::{Cursor, Dispatch, Progress, Resources};
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
pub fn serve(
    status: Arc<Status>,
    epoch: crate::system::identity::revision::Epoch,
    changed: crate::system::identity::revision::Changed,
) -> Result<(), Fail> {
    let mut resources = Resources::new();
    resources
        .insert(epoch)?
        .insert(changed)?
        .insert(status)?
        .insert(Book(None))?
        .insert(Faces(Vec::new()))?
        .insert(Pile::unseal(false)?)?
        .insert(Buffer(alloc::vec![0; env::PAGE_SIZE]))?
        .insert(Inbox(VecDeque::new()))?
        .insert(Current::Empty)?
        .insert(Ready(None))?
        .insert(Running(true))?
        .insert(Dispatch::<(), Fail>::new())?;
    let [mut start, mut frame, mut stop] = super::schedule::plans()?;
    start.prepare(&resources);
    frame.prepare(&resources);
    stop.prepare(&resources);
    start
        .advance(&mut Cursor::default(), &resources)
        .map_err(|_| Fail::Book)?;
    let mut cursor = Cursor::default();
    while resources.read::<Running>()?.0 {
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
