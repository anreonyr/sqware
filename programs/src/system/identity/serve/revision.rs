use runtime::schedule::{Progress, Res};
use super::{
    Fail,
    face::Current,
};
use core::sync::atomic::Ordering;
use env::pie;
use crate::system::identity::revision::{Epoch, Changed};

pub(super) fn publish(
    current: Res<Current>,
    epoch: Res<Epoch>,
) -> Result<Progress, Fail> {
    if current.mutated() {
        epoch.0.fetch_add(1, Ordering::Release);
    }
    Ok(Progress::Done)
}
pub(super) fn notify(
    current: Res<Current>,
    changed: Res<Changed>,
) -> Result<Progress, Fail> {
    if current.mutated() {
        match changed.0.ring() {
            Ok(()) => {}
            Err(error) if error.source.is_busy() => {}
            Err(_) => return Err(Fail::Desk),
        }
    }
    Ok(Progress::Done)
}
pub(super) fn close(changed: Res<Changed>) -> Result<Progress, Fail> {
    let _ = pie::release(changed.0.token());
    Ok(Progress::Done)
}
