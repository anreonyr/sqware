//! Device protocol selection and binding coordination in the independent Driver.

use alloc::vec::Vec;

use env::{HoleLimits, PieToken, Wait};
use hub_api::{fail_to_code, DmaCall, DmaSaid, DmaWire, Enroll, Fail, Grant, DMA_BACK, DMA_ENROLL};
use hub_api::coordination::{self, Call, Wire};
use ipc::{rpc::request::Receiver, session::establish};
use system_api::operator::Path;
use system_client::operator;

use crate::driver::iommu::api as iommu;
use crate::driver::iommu::domain_id::DomainId;
use crate::driver::pci::{api as pci, inspection};
use crate::driver::shared::{context::{Context, Step}, lookup, peer::Peer};
use crate::driver::virtio::Caps;
use crate::driver::virtio::blk::client;
use crate::driver::virtio::blk::coord::Coordinator;
use crate::driver::virtio::blk::peers::Peers;
use crate::driver::virtio::blk::profile::Profile;
use crate::driver::virtio::blk::recovery::executor::Executor;

const MS: usize = 2000;

pub fn serve() -> Result<(), &'static str> {
    let sire = env::unit::sire();
    let ctx = Context::open(sire, Wait::AtMost(MS)).map_err(|s| match s {
        Step::Tree => "virtio-blk: tree",
    })?;
    let tree = operator::Face::from(&ctx.session);
    let entry = env::hole::unseal(coordination::call::ENTRY, HoleLimits::default())
        .map_err(|_| "virtio-blk: entry")?;
    ctx.plate(entry, "coord", Wait::AtMost(MS));
    // Publishing readiness precedes peer discovery: authenticated publisher lookup
    // requires a live Ready slot, and Hub also discovers this coordination face.
    establish::lend(sire, crate::unit::READY_MARK).map_err(|_| "virtio-blk: ready")?;

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
    if config.host() != pci.host() { return Err("virtio-blk: PCI configuration owner"); }
    let iommu = loop {
        match lookup::peer::<iommu::Call>(&tree, &state, Path::new("iommu/entry"),
            iommu::ENTRY, iommu::Call::BACK, &mut held, Wait::AtMost(MS)) {
            Ok(peer) => break peer,
            Err(_) => pause(),
        }
    };
    let profiles = discover(&config)?;
    let peers = Peers { hub: hub.host(), pci, iommu };
    let mut coordinators = Vec::new();
    for profile in profiles {
        let domain = establish(&peers, profile.requester)?;
        enroll(&hub, profile.key)?;
        crate::debug::put(&alloc::format!(
            "virtio-blk: ready hub={} pci={} requester={} key={}",
            peers.hub.get(), peers.pci.host().get(), profile.requester, profile.key,
        ));
        let executor = Executor::new(peers.pci.host());
        coordinators.push(Coordinator::new(executor, profile, domain));
    }
    crate::debug::put(&alloc::format!(
        "virtio-blk: serving hub={} pci={} devices={}",
        peers.hub.get(), peers.pci.host().get(), coordinators.len(),
    ));
    run(entry, peers, coordinators)
}

fn discover(config: &Peer<inspection::Call>) -> Result<Vec<Profile>, &'static str> {
    let mut profiles = Vec::new();
    let mut cursor = 0;
    let mut next_key = 1u64;
    loop {
        let page = retry(|| {
            match config.call(Wait::AtMost(MS), |back| inspection::Scan { from: cursor, back }) {
                Ok(page) if page.status == wire::OK => Ok(Some(page)),
                Ok(_) => Err("virtio-blk: configuration refused"),
                Err(_) => Ok(None),
            }
        })?;
        if page.count != 0 {
            let vendor = u16::from_le_bytes([page.bytes[0], page.bytes[1]]);
            let device = u16::from_le_bytes([page.bytes[2], page.bytes[3]]);
            if vendor == 0x1af4 && matches!(device, 0x1001 | 0x1042) {
                let (bar, caps) = Caps::read(&page.bytes)?;
                let key = next_key;
                next_key = next_key.checked_add(1).ok_or("virtio-blk: key space")?;
                profiles.push(Profile {
                    key,
                    requester: page.requester,
                    bar,
                    caps: caps.parts(),
                });
            }
        }
        if page.end == inspection::EOF {
            break;
        }
        if page.next <= cursor {
            return Err("virtio-blk: scan did not advance");
        }
        cursor = page.next;
    }
    if profiles.is_empty() {
        return Err("virtio-blk: virtio block absent");
    }
    Ok(profiles)
}

fn establish(peers: &Peers, requester: u16) -> Result<DomainId, &'static str> {
    retry(|| {
        match peers.iommu.call(Wait::AtMost(MS), |back| iommu::Wire::Establish(iommu::Establish {
            op: iommu::ESTABLISH, requester: u32::from(requester), back,
        })) {
            Ok(said) if said.status == wire::OK
                && said.engine == peers.iommu.host() && said.key != 0 =>
                Ok(Some(DomainId::new(said.engine, said.key))),
            Ok(_) => Err("virtio-blk: domain refused"),
            Err(_) => Ok(None),
        }
    })
}

fn enroll(hub: &Peer<DmaCall>, key: u64) -> Result<(), &'static str> {
    let name = alloc::format!("virtio-blk-pci-{key:x}");
    retry(|| {
        match hub.call(Wait::AtMost(MS), |back| DmaWire::Enroll(Enroll {
            op: DMA_ENROLL, key, line: 0,
            name: name.as_str().into(), class: "virtio-blk".into(), back,
        })) {
            Ok(said) if said.status == wire::OK => Ok(Some(())),
            Ok(said) if said.status == hub_api::fail_to_code(Some(hub_api::Fail::Full)) => Ok(None),
            Ok(_) => Err("virtio-blk: enrollment denied"),
            Err(_) => Ok(None),
        }
    })
}

fn retry<T>(mut attempt: impl FnMut() -> Result<Option<T>, &'static str>) -> Result<T, &'static str> {
    loop {
        match attempt()? {
            Some(value) => return Ok(value),
            None => pause(),
        }
    }
}

fn run(entry: PieToken, peers: Peers, mut coordinators: Vec<Coordinator>) -> Result<(), &'static str> {
    let receiver = Receiver::<Call>::from_raw(entry, Call::BACK, Call::back);
    let mut buffer = [0u8; coordination::Wire::LEN];
    let mut client_buffer = [0u8; client::Wire::LEN];
    loop {
        for coordinator in &mut coordinators {
            if let Some((local, _)) = coordinator.operation.as_ref().and_then(|op| op.client) {
                let receiver = Receiver::<client::Call>::from_raw(local, client::BACK, client::Call::back);
                if let Ok(incoming) = receiver.receive(&mut client_buffer, Wait::POLL) {
                    let said = coordinator.client(&peers, incoming.from, incoming.request);
                    let _ = incoming.reply.send(said);
                }
            }
            coordinator.advance(&peers);
        }
        let incoming = match receiver.receive(&mut buffer, Wait::POLL) {
            Ok(incoming) => incoming,
            Err(rejected) => {
                if matches!(rejected.fail, ipc::rpc::Fail::Receive(
                    env::HoleFail::Dead | env::HoleFail::Denied
                )) { return Err("virtio-blk: door dead"); }
                pause();
                continue;
            }
        };
        let key = match incoming.request {
            Wire::Prepare(frame) => frame.key,
            Wire::Release(frame) => frame.key,
        };
        let said = match coordinators.iter_mut().find(|coord| coord.profile.key == key) {
            Some(coord) => coord.turn(&peers, incoming.from, incoming.request),
            None => DmaSaid::of(fail_to_code(Some(Fail::Unknown))),
        };
        let _ = incoming.reply.send(said);
    }
}

fn pause() {
    let _ = execution::room::park(::core::time::Duration::from_millis(1));
}
