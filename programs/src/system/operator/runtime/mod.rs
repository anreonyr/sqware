//! Operator owns the namespace, guest capabilities and subscriptions.
#[derive(Debug)]
pub enum Fail {
    Tree,
    Desk,
    Room,
    Dead,
}
mod admission;
mod answer;
mod door;
mod events;
mod exchange;
mod frame;
mod import;
mod management;
mod schedule;

use self::{
    answer::Output,
    door::Judgment,
    exchange::{Buffer, Hit, Outboxes, Request, Selected},
    frame::Running,
    management::{Ack, CurrentTip, Tip, Tips},
};
use crate::system::app::life::Status;
use crate::system::operator::session::Desk;
use crate::system::operator::tree::Operator;
use ::resource::pile::Pile;
use ::schedule::{Cursor, Dispatch, Progress, Resources};
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
use env::PieToken;
use system_api::operator as ocall;
use system_client::identity::TaskQuery;
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
        .insert(events::Watchers::new())
        .map_err(|_| Fail::Room)?;
    resources.insert(Desk::new()).map_err(|_| Fail::Room)?;
    resources
        .insert(None::<TaskQuery>)
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
    resources.insert(Running(true)).map_err(|_| Fail::Room)?;
    resources
        .insert(Dispatch::<(), Fail>::new())
        .map_err(|_| Fail::Room)?;
    let [mut start, mut frame, mut stop] = schedule::plans().map_err(|_| Fail::Room)?;
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
