use crate::hub_core::{Entry, Ledger, Owner};
use env::{Access, PieKind, PieToken, Policy, TaskId};
use hub_api::{self as hub, Deed};
#[path = "../../src/service/hub/serve/claim.rs"]
pub mod claim;
#[path = "../../src/service/hub/serve/sweep.rs"]
pub mod sweep;
fn put_deed(_: PieToken, deed: Deed) {
    crate::DEEDS.with(|d| d.borrow_mut().push(deed));
}
fn ship(_: Entry, _: TaskId, _: PieKind, _: Access, _: Policy) -> Result<PieToken, ()> {
    Ok(PieToken::mint(99))
}

pub(crate) fn claim_device(
    ledger: &mut Ledger,
    door: PieToken,
    from: TaskId,
    sensor: PieToken,
    kind: u8,
    access: u32,
    policy: u32,
    back: PieToken,
) {
    claim::claim(ledger, door, from, sensor, kind, access, policy, back);
}
pub(crate) fn alive(sensor: PieToken) -> bool {
    sweep::alive(sensor)
}
