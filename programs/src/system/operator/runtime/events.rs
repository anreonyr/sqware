//! Watch 请求接入与树变更到订阅事件的适配。
use crate::system::operator::tree::Operator;
use crate::system::operator::watch::Subscription;
pub(super) use crate::system::operator::watch::Watchers;
use ::schedule::{Progress, Res, ResMut};
use system_api::operator::Event;

pub fn event_at(tree: &Operator, change: crate::system::operator::tree::Change) -> Option<Event> {
    let road = match change.road {
        Some(road) => road,
        None => tree.road_to(change.id)?,
    };
    Some(Event {
        seq: 0,
        kind: change.kind,
        road,
        id: change.id,
        owner: change.owner,
    })
}

use super::{Fail, answer::Output, exchange::Request};
use system_api::operator as ocall;
pub(super) fn subscribe(
    mut request: ResMut<Request>,
    mut watchers: ResMut<Watchers>,
    mut out: ResMut<Output<ocall::Union>>,
) -> Result<Progress, super::Fail> {
    if let Some(incoming) = &mut request.0 {
        if matches!(incoming.ask, Some(ocall::Wire::Watch { .. })) {
            let Some(ocall::Wire::Watch { road, hole }) = incoming.ask.take() else {
                unreachable!();
            };
            out.reply = Some(
                match Subscription::import(incoming.guest.who(), road, hole)
                    .and_then(|subscription| watchers.join(subscription))
                {
                    Ok(()) => ocall::Union::Status(ocall::OK),
                    Err(()) => ocall::Union::Status(ocall::DENIED),
                },
            );
        }
    }
    Ok(Progress::Done)
}
pub(super) fn emit<T: 'static>(
    tree: Res<Operator>,
    mut watchers: ResMut<Watchers>,
    mut out: ResMut<Output<T>>,
) -> Result<Progress, Fail> {
    for change in out.changes.drain(..) {
        if let Some(event) = event_at(&tree, change) {
            let _ = watchers.publish(event);
        }
    }
    Ok(Progress::Done)
}
