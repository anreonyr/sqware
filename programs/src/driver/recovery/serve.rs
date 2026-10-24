//! Device protocol selection and binding coordination in the independent Driver.

use env::{HoleLimits, PieToken, Wait};
use hub_api::{DmaCall, DmaWire, Enroll, Grant, DMA_BACK, DMA_ENROLL};
use hub_api::coordination::{self, Call};
use ipc::{rpc::request::Receiver, session::establish};
use system_api::operator::Path;
use system_client::operator;

use crate::driver::pci::{api as pci, inspection};
use crate::driver::iommu::api as iommu;
use crate::driver::iommu::domain_id::DomainId;
use crate::driver::shared::{context::{Context, Step}, lookup, peer::Peer};
use crate::driver::virtio::Caps;
use super::{client, coord::Coordinator, core::Core, profile::Profile};

const MS: usize = 2000;

pub fn serve() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|s| match s {
        Step::Tree => "recovery: tree",
    })?;
    let tree = operator::Face::from(&ctx.session);
    let entry = env::hole::unseal(coordination::call::ENTRY, HoleLimits::default())
        .map_err(|_| "recovery: entry")?;
    ctx.plate(entry, "coord", Wait::AtMost(MS));
    // Publishing readiness precedes peer discovery: authenticated publisher lookup
    // requires a live Ready slot, and Hub also discovers this coordination face.
    establish::lend(sire, crate::unit::READY_MARK).map_err(|_| "recovery: ready")?;

    let mut held = None;
    let state = loop {
        match lookup::state(&tree, sire, &mut held, Wait::AtMost(MS)) {
            Ok(state) => break state,
            Err(_) => pause(),
        }
    };
    let hub = loop {
        match lookup::peer::<DmaCall>(&tree, &state, Path::new("hub/dma"),
            Grant::Dma.mark(), DMA_BACK, &mut held, Wait::AtMost(MS)) {
            Ok(peer) => break peer,
            Err(_) => pause(),
        }
    };
    let pci = loop {
        match lookup::peer::<pci::Call>(&tree, &state, Path::new("pci/entry"),
            pci::ENTRY, pci::Call::BACK, &mut held, Wait::AtMost(MS)) {
            Ok(peer) => break peer,
            Err(_) => pause(),
        }
    };
    let config = loop {
        match lookup::peer::<inspection::Call>(&tree, &state, Path::new(inspection::ENTRY_PATH),
            inspection::ENTRY, inspection::BACK, &mut held, Wait::AtMost(MS)) {
            Ok(peer) => break peer,
            Err(_) => pause(),
        }
    };
    if config.host() != pci.host() { return Err("recovery: PCI configuration owner"); }
    let iommu = loop {
        match lookup::peer::<iommu::Call>(&tree, &state, Path::new("iommu/entry"),
            iommu::ENTRY, iommu::Call::BACK, &mut held, Wait::AtMost(MS)) {
            Ok(peer) => break peer,
            Err(_) => pause(),
        }
    };
    let profile = discover(&config)?;
    let domain = loop {
        match iommu.call(Wait::AtMost(MS), |back| iommu::Wire::Establish(iommu::Establish {
            op: iommu::ESTABLISH, requester: u32::from(profile.requester), back,
        })) {
            Ok(said) if said.status == wire::OK && said.engine == iommu.host() && said.key != 0 =>
                break DomainId::new(said.engine, said.key),
            Ok(_) => return Err("recovery: domain refused"),
            Err(_) => pause(),
        }
    };
    loop {
        let said = hub.call(Wait::AtMost(MS), |back| DmaWire::Enroll(Enroll {
            op: DMA_ENROLL, key: profile.key, line: 0,
            name: "virtio-blk-pci".into(), class: "virtio-blk".into(), back,
        }));
        match said {
            Ok(said) if said.status == wire::OK => break,
            Ok(said) if said.status != hub_api::fail_to_code(Some(hub_api::Fail::Full)) =>
                return Err("recovery: enrollment denied"),
            _ => pause(),
        }
    }
    crate::debug::put(&alloc::format!(
        "recovery: ready hub={} pci={} requester={} key={}",
        hub.host().get(), pci.host().get(), profile.requester, profile.key,
    ));
    let core = Core::new(pci.host());
    run(entry, Coordinator::new(hub.host(), pci, iommu, core, profile, domain))
}

fn discover(config: &Peer<inspection::Call>) -> Result<Profile, &'static str> {
    let mut cursor = 0;
    loop {
        let page = config.call(Wait::AtMost(MS), |back| inspection::Scan { from: cursor, back })
            .map_err(|_| "recovery: configuration scan")?;
        if page.status != wire::OK { return Err("recovery: configuration refused"); }
        if page.count != 0 {
            let vendor = u16::from_le_bytes([page.bytes[0], page.bytes[1]]);
            let device = u16::from_le_bytes([page.bytes[2], page.bytes[3]]);
            if vendor == 0x1af4 && matches!(device, 0x1001 | 0x1042) {
                let (bar, caps) = Caps::read(&page.bytes)?;
                return Ok(Profile { key: 1, requester: page.requester, bar, caps: caps.parts() });
            }
        }
        if page.end == inspection::EOF { return Err("recovery: virtio block absent"); }
        if page.next <= cursor { return Err("recovery: scan did not advance"); }
        cursor = page.next;
    }
}

fn run(entry: PieToken, mut coordinator: Coordinator) -> Result<(), &'static str> {
    let receiver = Receiver::<Call>::from_raw(entry, Call::BACK, Call::back);
    let mut buffer = [0u8; coordination::Wire::LEN];
    let mut client_buffer = [0u8; client::Wire::LEN];
    loop {
        if let Some((local, _)) = coordinator.operation.as_ref().and_then(|op| op.client) {
            let receiver = Receiver::<client::Call>::from_raw(local, client::BACK, client::Call::back);
            if let Ok(incoming) = receiver.receive(&mut client_buffer, Wait::POLL) {
                let said = coordinator.client(incoming.from, incoming.request);
                let _ = incoming.reply.send(said);
            }
        }
        let incoming = match receiver.receive(&mut buffer, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rejected) => {
                if matches!(rejected.fail, ipc::rpc::Fail::Receive(
                    env::HoleFail::Dead | env::HoleFail::Denied
                )) { return Err("recovery: door dead"); }
                pause();
                continue;
            }
        };
        let said = coordinator.turn(incoming.from, incoming.request);
        let _ = incoming.reply.send(said);
    }
}

fn pause() {
    let _ = execution::room::park(::core::time::Duration::from_millis(1));
}
