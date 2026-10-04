use super::{
    Fail,
    face::{Current, Response},
};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use env::pie;
use runtime::core::res::bell::Bell;

use protocol::{
    common::schedule::{Progress, Res},
    system::identity::{Mount, Reply},
};
#[derive(Clone)]
pub struct Epoch(pub Arc<AtomicU64>);
pub struct Changed(pub Bell);
impl Epoch {
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }
}
fn mutated(current: &Current, response: &Response) -> bool {
    current
        .0
        .as_ref()
        .is_some_and(|incoming| incoming.request.grant.mount() != Mount::Public)
        && matches!(response.0, Some(reply) if !matches!(reply, Reply::Fail(_)))
}
pub(super) fn publish(
    current: Res<Current>,
    response: Res<Response>,
    epoch: Res<Epoch>,
) -> Result<Progress, Fail> {
    if mutated(&current, &response) {
        epoch.0.fetch_add(1, Ordering::Release);
    }
    Ok(Progress::Done)
}
pub(super) fn notify(
    current: Res<Current>,
    response: Res<Response>,
    changed: Res<Changed>,
) -> Result<Progress, Fail> {
    if mutated(&current, &response) {
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
