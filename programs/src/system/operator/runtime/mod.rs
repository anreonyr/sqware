//! Operator owns the namespace, guest capabilities and subscriptions.
#[derive(Debug)]
pub enum Fail {
    Tree,
    Desk,
    Room,
    Dead,
}
impl From<::schedule::resource::AccessError> for Fail {
    fn from(_: ::schedule::resource::AccessError) -> Self {
        Self::Room
    }
}
impl From<::schedule::BuildError> for Fail {
    fn from(_: ::schedule::BuildError) -> Self {
        Self::Room
    }
}
impl From<::schedule::DispatchError> for Fail {
    fn from(_: ::schedule::DispatchError) -> Self {
        Self::Room
    }
}
impl From<alloc::collections::TryReserveError> for Fail {
    fn from(_: alloc::collections::TryReserveError) -> Self {
        Self::Room
    }
}
impl From<erra::Error<env::PieFail>> for Fail {
    fn from(_: erra::Error<env::PieFail>) -> Self {
        Self::Desk
    }
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
    resources
        .insert(status)?
        .insert(Tip(PieToken::NONE))?
        .insert(Pile::unseal(false)?)?
        .insert(Operator::new())?
        .insert(events::Watchers::new())?
        .insert(Desk::new())?
        .insert(None::<TaskQuery>)?
        .insert(Outboxes(Vec::new()))?
        .insert(Tips(VecDeque::new()))?
        .insert(CurrentTip(None))?
        .insert(Output::<Ack> {
            reply: None,
            changes: Vec::new(),
        })?
        .insert(Output::<ocall::Union> {
            reply: None,
            changes: Vec::new(),
        })?
        .insert(Request(None))?
        .insert(Judgment(None))?
        .insert(Buffer(alloc::vec![0; env::PAGE_SIZE]))?
        .insert(Hit(None))?
        .insert(Selected(None))?
        .insert(Running(true))?
        .insert(Dispatch::<(), Fail>::new())?;
    let [mut start, mut frame, mut stop] = schedule::plans()?;
    start.prepare(&resources);
    frame.prepare(&resources);
    stop.prepare(&resources);
    start
        .advance(&mut Cursor::default(), &resources)
        .map_err(|_| Fail::Tree)?;
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
