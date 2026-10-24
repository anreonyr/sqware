//! Device enrollment, claims and logical binding responsibility.

use env::{PieToken, TaskId};
use hub_api::{Close, DmaSaid, DmaWire, Fail};

use crate::service::hub::core::{ClaimRequest, Ledger};
use super::directory::Directory;
use super::peers::Peers;
use super::claim;

fn code(fail: Fail) -> u8 {
    hub_api::fail_to_code(Some(fail))
}

pub fn turn(
    ledger: &mut Ledger,
    peers: Option<&Peers>,
    directory: &Directory<'_>,
    from: TaskId,
    request: DmaWire,
) -> DmaSaid {
    match request {
        DmaWire::Enroll(enroll) => {
            let Some(peers) = peers else {
                return DmaSaid::of(code(Fail::Full));
            };
            if peers.coord.host() != from {
                return DmaSaid::of(code(Fail::Denied));
            }
            let device = hub_api::DeviceId { source: from, key: enroll.key };
            if let Err(fail) = ledger.prepare_enrollment(from, enroll) {
                return DmaSaid::of(code(fail));
            }
            match ledger.advance_enrollment(device, &mut &*directory) {
                Some(Ok(())) => DmaSaid::of(wire::OK),
                Some(Err(fail)) => {
                    crate::debug::put(&alloc::format!("hub: enrollment source={} key={} failed {fail:?} (holding)",
                        device.source.get(), device.key));
                    DmaSaid::of(code(Fail::Denied))
                }
                None => DmaSaid::of(code(Fail::Unknown)),
            }
        }
        DmaWire::Claim(_) => DmaSaid::of(code(Fail::Denied)),
        DmaWire::Close(close) => {
            if !ledger.binding(close.serial).is_some_and(|binding| binding.execution == from) {
                return DmaSaid::of(code(Fail::Denied));
            }
            match Close::of_code(close.reason) {
                Some(reason) => match ledger.close(close.serial, reason) {
                    Ok(state) => DmaSaid::bound(close.serial, state),
                    Err(fail) => DmaSaid::of(code(fail)),
                },
                None => DmaSaid::of(code(Fail::Bad)),
            }
        }
        DmaWire::Inspect(inspect) => match ledger.binding(inspect.serial) {
            Some(binding) => DmaSaid::progress(binding.id.serial, binding.state, binding.failure),
            None => DmaSaid::of(code(Fail::Unknown)),
        },
        DmaWire::Settle(_) if from != env::unit::sire() => DmaSaid::of(code(Fail::Denied)),
        DmaWire::Settle(_) => {
            let open = ledger.settle();
            let mut said = DmaSaid::of(wire::OK);
            said.count = u8::try_from(open).unwrap_or(u8::MAX);
            said
        }
    }
}

pub fn claim_at_door(ledger: &mut Ledger, door: PieToken, from: TaskId, frame: &[u8]) -> Option<DmaSaid> {
    let (claim, end) = <hub_api::DmaClaim as env::wire::Span>::fetch_at(frame, 0)?;
    if claim.op != hub_api::DMA_CLAIM || end != frame.len() {
        return None;
    }
    let said = match ledger.device_by_door(door) {
        None => DmaSaid::of(code(Fail::Unknown)),
        Some(_) if !claim::sensor_from(from, claim.sensor) => DmaSaid::of(code(Fail::Denied)),
        Some(device) => match ledger.claim(ClaimRequest {
            from, id: claim.id, device, sensor: claim.sensor,
        }) {
            Ok(id) => {
                let binding = ledger.binding(id.serial).expect("claim receipt");
                DmaSaid::progress(id.serial, binding.state, binding.failure)
            }
            Err(fail) => DmaSaid::of(code(fail)),
        },
    };
    let mut tx = ipc::hand::Sender::<DmaSaid>::from_raw(claim.back);
    let _ = tx.send(said);
    Some(said)
}
