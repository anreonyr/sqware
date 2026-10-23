use core::time::Duration;

use ::resource::raw::{Hole, reserve};
use env::{Access, HoleLimits, Wait, pie};
use execution::unit::task;
use ipc::session::establish;
use system_api::operator::{OperationId, Permit, ResourceView};
use system_client::operator::Capability;

use super::api;
use super::ask::Ask;
use super::beat::Beat;

const MS: usize = 3000;

pub fn run() -> Result<(), &'static str> {
    let wait = Wait::AtMost(MS);
    let sire = env::unit::sire();
    let capability =
        Capability::injected(0, sire, wait).map_err(|_| "fixturetarget: operator capability")?;
    let entry = env::hole::unseal(api::ENTRY_MARK, HoleLimits::default())
        .map_err(|_| "fixturetarget: entry hole")?;
    let anchor = capability
        .anchor()
        .map_err(|_| "fixturetarget: namespace anchor")?;
    let view = ResourceView::new(Access::STORE).ok_or("fixturetarget: entry view")?;
    capability
        .bind(
            anchor,
            api::ENTRY_NAME,
            entry,
            view,
            Permit::Public,
            OperationId(1),
            wait,
        )
        .map_err(|_| "fixturetarget: entry binding")?;

    let helper = task::spawn(|| loop {
        let _ = execution::room::park(Duration::from_millis(1000));
    });
    let helper_id = helper.id();
    let me = env::unit::self_id();
    establish::lend(sire, crate::unit::READY_MARK).map_err(|_| "fixturetarget: ready")?;

    let mut bytes = [0u8; Ask::LEN];
    loop {
        let Ok((len, from)) = Hole::from_raw(entry).pull(&mut bytes, Wait::Forever) else {
            return Err("fixturetarget: entry pull");
        };
        let Some(ask) = Ask::decode(&bytes[..len]) else {
            continue;
        };
        let Ok((vestor, owner, mark)) = reserve(ask.back) else {
            continue;
        };
        if ask.op != api::ASK
            || ask.target != me
            || vestor != from
            || owner != from
            || mark != api::BACK
        {
            let _ = pie::release(ask.back, env::ReleaseMode::Revoke);
            continue;
        }
        if !matches!(env::unit::join_task(helper_id, Wait::POLL), Ok(false)) {
            let _ = pie::release(ask.back, env::ReleaseMode::Revoke);
            continue;
        }
        let beat = Beat::of(me, helper_id, env::chrono::clock());
        let mut out = [0u8; Beat::LEN];
        let Some(n) = beat.encode(&mut out) else {
            let _ = pie::release(ask.back, env::ReleaseMode::Revoke);
            continue;
        };
        let _ = Hole::from_raw(ask.back).push(&out[..n], Wait::AtMost(MS));
        let _ = pie::release(ask.back, env::ReleaseMode::Revoke);
    }
}
