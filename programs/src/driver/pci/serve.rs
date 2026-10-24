//! Bounded configuration snapshots and authenticated PCI resource effects.

use env::{HoleCondition, HoleLimits, TaskId, ToleAwaitReply, ToleEventSource, Wait};
use ipc::{rpc::{self, request::Receiver}, session::establish};
use resource::pile::Pile;
use system_api::operator::{OperationId, Permit, ResourceView};
use system_client::operator::{self, Capability};

use super::{api::{self, Call}, bindings::{APERTURE_BINDING, APERTURE_SUBSET, ECAM_BINDING, ECAM_SUBSET},
    core::{self, Core, PlaceRequest}, inspection, window::EcamWindow};
use crate::driver::shared::{context::{Context, Step}, lookup, platform};

const MS: usize = 2000;
const BATCH: usize = 8;

pub fn serve() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let ecam = platform::take(sire, ECAM_BINDING, ECAM_SUBSET, Wait::AtMost(MS))?;
    let aperture = platform::take(sire, APERTURE_BINDING, APERTURE_SUBSET, Wait::AtMost(MS))?;
    let dock = core::open_ecam(&ecam)?;
    let base = dock.view().base();
    // The dock remains in this frame throughout the server loop.
    let window = unsafe { EcamWindow::new(base, dock.view().size()) }.ok_or("pci: ecam geometry")?;
    let mut core = Core::new(base, window.limit(), aperture)?;
    crate::debug::put(&alloc::format!(
        "pci: space task={} ecam={base:#x} functions={}", env::unit::self_id().get(), window.limit()
    ));
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|Step::Tree| "pci: tree")?;
    let tree = operator::Face::from(&ctx.session);
    let entry = env::hole::unseal(api::ENTRY, HoleLimits::default()).map_err(|_| "pci: entry")?;
    ctx.plate(entry, api::ENTRY_NAME, Wait::AtMost(MS));
    let config = env::hole::unseal(inspection::ENTRY, HoleLimits::default()).map_err(|_| "pci: config entry")?;
    let publisher = Capability::injected(1, sire, Wait::AtMost(MS)).map_err(|_| "pci: config publisher")?;
    let anchor = publisher.anchor().map_err(|_| "pci: config anchor")?;
    publisher.bind(anchor, "config", config, ResourceView::new(env::Access::STORE).unwrap(),
        Permit::Public, OperationId(2), Wait::AtMost(MS)).map_err(|_| "pci: config publication")?;
    let pile = Pile::unseal(false).map_err(|_| "pci: pile")?;
    for token in [entry, config] {
        pile.attach(ToleEventSource::Hole { token, condition: HoleCondition::Pull })
            .map_err(|_| "pci: pile attach")?;
    }
    establish::lend(sire, crate::unit::READY_MARK).map_err(|_| "pci: ready delivery")?;
    crate::debug::put(&alloc::format!("pci: ready functions={}", window.limit()));
    let effects = Receiver::<Call>::from_raw(entry, Call::BACK, Call::back);
    let snapshots = Receiver::<inspection::Call>::from_raw(config, inspection::Call::BACK, inspection::Call::back);
    let mut buffer = [0u8; api::Wire::LEN];
    let mut state = None;
    let mut rejected_state = None;
    let mut coordinator = None;
    loop {
        lookup::reclaim(&mut rejected_state);
        let source = match pile.await_(Wait::Forever).map_err(|_| "pci: pile dead")? {
            ToleAwaitReply::Source { source: ToleEventSource::Hole { token, .. }, .. } => token,
            _ => continue,
        };
        if source == config {
            for _ in 0..BATCH {
                match snapshots.receive(&mut buffer, Wait::POLL) {
                    Ok(incoming) => {
                        let said = inspection::enumerate::scan(&window, incoming.request.from);
                        let _ = incoming.reply.send(said);
                    }
                    Err(rejected) => match rejected.fail {
                        rpc::Fail::Receive(error) if matches!(error, env::HoleFail::Dead | env::HoleFail::Denied) =>
                            return Err("pci: config door dead"),
                        rpc::Fail::Receive(_) => break,
                        _ => continue,
                    }
                }
            }
        } else if source == entry {
            for _ in 0..BATCH {
                match effects.receive(&mut buffer, Wait::POLL) {
                    Ok(incoming) => {
                        if state.is_none() {
                            state = lookup::state(&tree, sire, &mut rejected_state, Wait::AtMost(MS)).ok();
                        }
                        if coordinator.is_none() {
                            coordinator = state.as_ref().and_then(|state| state.publisher(
                                system_api::namespace::SERVICE, hub_api::coordination::call::ENTRY, Wait::AtMost(MS)
                            ).ok());
                        }
                        let said = if coordinator == Some(incoming.from) {
                            turn(&mut core, incoming.from, incoming.request)
                        } else { api::Said::of(api::fail_to_code(Some(api::Fail::Denied))) };
                        let _ = incoming.reply.send(said);
                    }
                    Err(rejected) => match rejected.fail {
                        rpc::Fail::Receive(error) if matches!(error, env::HoleFail::Dead | env::HoleFail::Denied) =>
                            return Err("pci: effect door dead"),
                        rpc::Fail::Receive(_) => break,
                        _ => continue,
                    }
                }
            }
        }
    }
}

fn turn(core: &mut Core, from: TaskId, request: api::Wire) -> api::Said {
    match request {
        api::Wire::Place(place) => match core.place(PlaceRequest {
            serial: place.serial, requester: place.requester, recipient: from, bar: place.bar,
        }) {
            Ok(placement) => api::Said { status: wire::OK, pages: placement.pages(), bar: placement.bar },
            Err(fail) => api::Said::of(api::fail_to_code(Some(fail))),
        },
        api::Wire::Enable(toggle) => result(core.enable(toggle.serial, toggle.requester)),
        api::Wire::Disable(toggle) => result(core.disable(toggle.serial, toggle.requester)),
        api::Wire::Release(toggle) => result(core.release(toggle.serial, toggle.requester)),
    }
}

fn result(reply: Result<(), api::Fail>) -> api::Said {
    api::Said::of(match reply { Ok(()) => wire::OK, Err(fail) => api::fail_to_code(Some(fail)) })
}
