use ::schedule::{Cursor, Dispatch, Progress, Resources};
use super::{
    Fail, answer::Output, watch, door::Judgment, frame::Running,
    session::{Buffer, Hit, LateGuests, Outboxes, Request, Selected, Settling},
    tip::{Ack, CurrentTip, Tip, Tips},
};
use crate::system::{common::face::desk::Desk, life::Status, operator::core::Operator};
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use env::PieToken;
use protocol::system::identity::client::TaskQuery;
use system_api::operator as ocall;
use ::resource::pile::Pile;
pub fn serve(status: Arc<Status>) -> Result<(), Fail> {
    let mut resources = Resources::new();
    resources.insert(status).map_err(|_| Fail::Room)?;
    resources
        .insert(Tip(PieToken::NONE))
        .map_err(|_| Fail::Room)?;
    resources
        .insert(Pile::unseal(false).map_err(|_| Fail::Desk)?)
        .map_err(|_| Fail::Room)?;
    resources.insert(Operator::new()).map_err(|_| Fail::Room)?;
    resources
        .insert(watch::Watchers::new())
        .map_err(|_| Fail::Room)?;
    resources.insert(Desk::new()).map_err(|_| Fail::Room)?;
    resources
        .insert(None::<TaskQuery>)
        .map_err(|_| Fail::Room)?;
    resources
        .insert(LateGuests(Vec::new()))
        .map_err(|_| Fail::Room)?;
    resources
        .insert(Outboxes(Vec::new()))
        .map_err(|_| Fail::Room)?;
    resources
        .insert(Tips(VecDeque::new()))
        .map_err(|_| Fail::Room)?;
    resources.insert(CurrentTip(None)).map_err(|_| Fail::Room)?;
    resources
        .insert(Output::<Ack> {
            reply: None,
            changes: Vec::new(),
        })
        .map_err(|_| Fail::Room)?;
    resources
        .insert(Output::<ocall::Union> {
            reply: None,
            changes: Vec::new(),
        })
        .map_err(|_| Fail::Room)?;
    resources.insert(Request(None)).map_err(|_| Fail::Room)?;
    resources.insert(Judgment(None)).map_err(|_| Fail::Room)?;
    resources
        .insert(Buffer(alloc::vec![0; env::PAGE_SIZE]))
        .map_err(|_| Fail::Room)?;
    resources.insert(Hit(None)).map_err(|_| Fail::Room)?;
    resources.insert(Selected(None)).map_err(|_| Fail::Room)?;
    resources.insert(Settling(false)).map_err(|_| Fail::Room)?;
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
        .map_err(|_| Fail::Tree)?;
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
