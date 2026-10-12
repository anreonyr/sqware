//! Watch 请求接入与树变更到订阅事件的适配。
use crate::system::operator::tree::Operator;
use crate::system::operator::watch::Subscription;
pub(super) use crate::system::operator::watch::Watchers;
use ::schedule::{Progress, ResMut};
use system_api::operator::{Entry, EntryTarget, Event, Path};

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
    tree: ResMut<Operator>,
    mut out: ResMut<Output>,
) -> Result<Progress, super::Fail> {
    if let Some(incoming) = &mut request.0 {
        if matches!(
            incoming.ask,
            Some(ocall::Wire::Watch { .. } | ocall::Wire::WatchFrom { .. })
        ) {
            let Some(ask) = incoming.ask.take() else {
                unreachable!();
            };
            let (base, road, hole) = match ask {
                ocall::Wire::Watch { road, hole } => (None, road, hole),
                ocall::Wire::WatchFrom { base, road, hole } => (Some(base), road, hole),
                _ => unreachable!(),
            };
            let who = incoming.guest.who();
            out.reply = Some(match base {
                None => match Subscription::import(who, road, hole)
                    .and_then(|subscription| out.watchers.join(subscription))
                {
                    Ok(()) => ocall::Union::Status(ocall::OK),
                    Err(()) => ocall::Union::Status(ocall::DENIED),
                },
                Some(base) => {
                    let root = Entry {
                        operator: base.operator,
                        target: EntryTarget::Root,
                    };
                    match tree.seek_from(base, Path::ROOT) {
                        Err(fail) => ocall::Union::Status(ocall::fail_to_code(Some(fail))),
                        Ok(_) => match tree.relative_to(root, base) {
                            Some(base_path) => match base_path.as_path().try_join(road.as_str()) {
                                Some(canonical) => {
                                    match Subscription::import(who, canonical, hole)
                                        .and_then(|subscription| out.watchers.join(subscription))
                                    {
                                        Ok(()) => ocall::Union::WatchPath(base_path),
                                        Err(()) => ocall::Union::Status(ocall::DENIED),
                                    }
                                }
                                None => ocall::Union::Status(ocall::FULL),
                            },
                            None => ocall::Union::Status(ocall::UNKNOWN),
                        },
                    }
                }
            });
        }
    }
    Ok(Progress::Done)
}
pub(super) fn emit(mut tree: ResMut<Operator>, mut out: ResMut<Output>) -> Result<Progress, Fail> {
    let changes = core::mem::take(&mut out.changes);
    for change in changes {
        if let Some(event) = event_at(&tree, change) {
            let _ = out.watchers.publish(event);
        }
    }
    for change in tree.take_changes() {
        if let Some(event) = event_at(&tree, change) {
            let _ = out.watchers.publish(event);
        }
    }
    Ok(Progress::Done)
}
