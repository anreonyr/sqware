//! Concrete coordination cursor owned by one physical device.

use env::{TaskId, Wait};
use hub_api::coordination::{Prepare, Release, Wire};
use hub_api::{
    fail_to_code, Close, DmaSaid, Fail, PARTY_IOMMU, PARTY_PCI, PARTY_RECOVERY,
};
use resource::raw::Hole;
use wire::Message;

use crate::driver::iommu::api as iommu;
use crate::driver::iommu::domain_id::DomainId;
use crate::driver::pci::api as pci;
use crate::driver::virtio::blk::client;
use crate::driver::virtio::blk::exchange::Exchange;
use crate::driver::virtio::blk::operation::Operation;
use crate::driver::virtio::blk::peers::Peers;
use crate::driver::virtio::blk::polled::Polled;
use crate::driver::virtio::blk::profile::Profile;
use crate::driver::virtio::blk::recovery::api;
use crate::driver::virtio::blk::recovery::attach_request::AttachRequest;
use crate::driver::virtio::blk::recovery::executor::Executor;
use crate::driver::virtio::blk::stage::Stage;
use crate::driver::shared::dma::{self, Coords};

const MS: usize = 3000;

pub struct Coordinator {
    pub executor: Executor,
    pub operation: Option<Operation>,
    pub last_serial: u64,
    pub profile: Profile,
    pub domain: DomainId,
}

impl Coordinator {
    pub fn new(executor: Executor, profile: Profile, domain: DomainId) -> Self {
        Self {
            executor,
            operation: None,
            last_serial: 0,
            profile,
            domain,
        }
    }

    pub fn turn(&mut self, peers: &Peers, from: TaskId, wire: Wire) -> DmaSaid {
        match wire {
            Wire::Prepare(frame) => {
                if from != peers.hub || frame.hub != from {
                    return DmaSaid::of(fail_to_code(Some(Fail::Denied)));
                }
                if let Err(fail) = self.begin(frame) {
                    return DmaSaid::of(fail_to_code(Some(fail)));
                }
            }
            Wire::Release(frame) => {
                if from != peers.hub || frame.hub != from {
                    return DmaSaid::of(fail_to_code(Some(Fail::Denied)));
                }
                if let Err(fail) = self.begin_release(frame) {
                    return DmaSaid::of(fail_to_code(Some(fail)));
                }
            }
        }
        self.reply()
    }

    /// One bounded step: poll the retained exchange, or start one local effect
    /// or one peer request. A pending exchange keeps the encoded stage intact.
    pub fn advance(&mut self, peers: &Peers) -> bool {
        let Some(stage) = self.operation.as_ref().map(|op| op.stage) else {
            return false;
        };
        if matches!(stage, Stage::Active | Stage::Done) { return false; }
        match self.poll_exchange() {
            Polled::Empty => {}
            Polled::Pci(Ok(None)) | Polled::Iommu(Ok(None)) => return false,
            Polled::Pci(Ok(Some(said))) => return self.complete_pci(stage, said),
            Polled::Iommu(Ok(Some(said))) => return self.complete_iommu(stage, said),
            Polled::Pci(Err(())) => {
                self.fail(PARTY_PCI, pci_op(stage), 0);
                self.reconcile_close();
                return false;
            }
            Polled::Iommu(Err(())) => {
                self.fail(PARTY_IOMMU, iommu_op(stage), 0);
                self.reconcile_close();
                return false;
            }
        }
        if self.operation.as_ref().is_some_and(|op| op.closing) && !is_cleanup(stage) {
            self.stage(Stage::Invalidate);
            return true;
        }
        if !self.operation.as_ref().is_some_and(|op| op.closing)
            && self.operation.as_ref().is_some_and(|op| op.client.is_none())
        {
            return self.client_entry();
        }
        match stage {
            Stage::Place => self.begin_place(peers),
            Stage::Attach => self.attach(),
            Stage::Acquire => self.begin_acquire(peers),
            Stage::Publish => self.begin_publish(peers),
            Stage::Enable => self.begin_enable(peers),
            Stage::Handoff => self.handoff(),
            Stage::Invalidate => self.begin_invalidate(peers),
            Stage::Disable => self.begin_disable(peers),
            Stage::Stop => self.stop(),
            Stage::Quiesce => self.begin_quiesce(peers),
            Stage::Release => self.begin_iommu_release(peers),
            Stage::Detach => self.detach(),
            Stage::CloseHandoff => self.close_handoff(),
            Stage::Unplace => self.begin_unplace(peers),
            Stage::Active | Stage::Done => false,
        }
    }

    pub fn client(&mut self, peers: &Peers, from: TaskId, request: client::Wire) -> client::Said {
        let refused = |fail| client::Said {
            status: fail_to_code(Some(fail)),
            iommu: TaskId::new(0),
        };
        let Some(op) = self.operation.as_mut() else { return refused(Fail::Unknown); };
        let identity = request.identity();
        if from != op.execution || identity.hub != op.hub || identity.serial != op.serial {
            return refused(Fail::Denied);
        }
        if !matches!(op.state(), hub_api::BindingState::Preparing | hub_api::BindingState::Active) {
            return refused(Fail::Conflict);
        }
        if let client::Wire::Supply(frame) = request {
            let source = (frame.source, frame.access);
            if frame.source == env::PieToken::NONE
                || frame.access != env::Access::FETCH_STORE.bits().bits()
            {
                return refused(Fail::Denied);
            }
            if op.source.is_some_and(|previous| previous != source) {
                return refused(Fail::Conflict);
            }
            if op.source.is_none() && op.state() != hub_api::BindingState::Preparing {
                return refused(Fail::Conflict);
            }
            op.source = Some(source);
        }
        client::Said { status: wire::OK, iommu: peers.iommu.host() }
    }

    fn begin(&mut self, frame: Prepare) -> Result<(), Fail> {
        if frame.key != self.profile.key {
            return Err(Fail::Denied);
        }
        if let Some(op) = self.operation.as_ref() {
            if op.hub == frame.hub
                && op.serial == frame.serial
                && op.execution == frame.execution
                && op.key == frame.key
            {
                return if op.stage == Stage::Done { Err(Fail::Conflict) } else { Ok(()) };
            }
            if op.stage != Stage::Done || frame.serial <= self.last_serial {
                return Err(Fail::Conflict);
            }
        }
        let retained_bar = self.operation.as_ref().and_then(|op| op.bar);
        self.operation = Some(Operation::new(
            frame.hub,
            frame.serial,
            frame.execution,
            frame.key,
        ));
        let op = self.operation.as_mut().unwrap();
        op.bar = retained_bar;
        op.caps = self.profile.caps;
        Ok(())
    }

    fn begin_release(&mut self, frame: Release) -> Result<(), Fail> {
        let reason = Close::of_code(frame.reason).ok_or(Fail::Bad)?;
        if frame.key != self.profile.key { return Err(Fail::Unknown); }
        if !self.operation.as_ref().is_some_and(|op| op.serial == frame.serial) {
            if self.operation.as_ref().is_some_and(|op| op.stage != Stage::Done)
                || frame.serial <= self.last_serial
            {
                return Err(Fail::Conflict);
            }
            let retained_bar = self.operation.as_ref().and_then(|op| op.bar);
            self.operation = Some(Operation::new(frame.hub, frame.serial, frame.execution, frame.key));
            self.operation.as_mut().unwrap().bar = retained_bar;
        }
        let op = self.operation.as_mut().unwrap();
        if op.hub != frame.hub || op.serial != frame.serial || op.execution != frame.execution || op.key != frame.key {
            return Err(Fail::Conflict);
        }
        if reason == Close::Fault {
            op.reason = Close::Fault;
        }
        if op.stage == Stage::Done {
            return Ok(());
        }
        op.closing = true;
        if op.exchange.is_none() && !is_cleanup(op.stage) {
            op.stage = Stage::Invalidate;
        }
        Ok(())
    }

    fn poll_exchange(&mut self) -> Polled {
        let Some(op) = self.operation.as_mut() else { return Polled::Empty; };
        let Some(exchange) = op.exchange.as_mut() else { return Polled::Empty; };
        let outcome = match exchange {
            Exchange::Pci(pending) => Polled::Pci(pending.poll().map_err(|_| ())),
            Exchange::Iommu(pending) => Polled::Iommu(pending.poll().map_err(|_| ())),
        };
        let terminal = !matches!(outcome, Polled::Pci(Ok(None)) | Polled::Iommu(Ok(None)));
        if terminal {
            op.exchange = None;
        }
        outcome
    }

    fn client_entry(&mut self) -> bool {
        let op = self.operation.as_ref().unwrap();
        if op.client.is_some() { return true; }
        match ipc::session::establish::lend(op.execution, client::ENTRY) {
            Ok(endpoint) => {
                let op = self.operation.as_mut().unwrap();
                op.client = Some((endpoint.rx(), endpoint.seed()));
                true
            }
            Err(_) => {
                self.fail(PARTY_RECOVERY, api::ATTACH, 0);
                false
            }
        }
    }

    fn begin_pci(
        &mut self,
        peers: &Peers,
        party: u8,
        operation: u8,
        build: impl FnOnce(env::PieToken) -> pci::Wire,
    ) -> bool {
        match peers.pci.begin(Wait::AtMost(MS), build) {
            Ok(pending) => {
                if let Some(op) = self.operation.as_mut() {
                    op.exchange = Some(Exchange::Pci(pending));
                }
                true
            }
            Err(_) => {
                self.fail(party, operation, 0);
                self.reconcile_close();
                false
            }
        }
    }

    fn begin_iommu(
        &mut self,
        peers: &Peers,
        party: u8,
        operation: u8,
        build: impl FnOnce(env::PieToken) -> iommu::Wire,
    ) -> bool {
        match peers.iommu.begin(Wait::AtMost(MS), build) {
            Ok(pending) => {
                if let Some(op) = self.operation.as_mut() {
                    op.exchange = Some(Exchange::Iommu(pending));
                }
                true
            }
            Err(_) => {
                self.fail(party, operation, 0);
                self.reconcile_close();
                false
            }
        }
    }

    fn begin_place(&mut self, peers: &Peers) -> bool {
        let Some(op) = self.operation.as_ref() else { return false; };
        let (serial, requester, bar) = (op.serial, self.profile.requester, self.profile.bar);
        self.begin_pci(peers, PARTY_PCI, pci::PLACE, |back| {
            pci::Wire::Place(pci::Place {
                op: pci::PLACE,
                serial,
                requester: u32::from(requester),
                bar,
                back,
            })
        })
    }

    fn attach(&mut self) -> bool {
        let Some(op) = self.operation.as_ref() else {
            return false;
        };
        let Some(bar) = op.bar else {
            self.fail(PARTY_RECOVERY, api::ATTACH, 0);
            return false;
        };
        let request = AttachRequest {
            serial: op.serial,
            execution: op.execution,
            bar,
            pages: op.pages,
            common: op.caps.0,
            notify: op.caps.1,
            multiplier: op.caps.2,
            device: op.caps.3,
        };
        match self.executor.attach(request) {
            Ok(()) => {
                self.stage(Stage::Acquire);
                true
            }
            Err(fail) => {
                self.fail(PARTY_RECOVERY, api::ATTACH, api::fail_to_code(Some(fail)));
                false
            }
        }
    }

    fn begin_acquire(&mut self, peers: &Peers) -> bool {
        let Some(op) = self.operation.as_ref() else { return false; };
        let Some((source, access)) = op.source else { return false; };
        let (hub, serial, execution) = (op.hub, op.serial, op.execution);
        let (engine, key) = (self.domain.engine(), self.domain.key());
        self.begin_iommu(peers, PARTY_IOMMU, iommu::ACQUIRE, |back| {
            iommu::Wire::Acquire(iommu::Acquire {
                op: iommu::ACQUIRE,
                engine,
                key,
                hub,
                serial,
                execution,
                source,
                access,
                back,
            })
        })
    }

    fn begin_publish(&mut self, peers: &Peers) -> bool {
        let Some(op) = self.operation.as_ref() else { return false; };
        let (hub, serial) = (op.hub, op.serial);
        let (engine, key) = (self.domain.engine(), self.domain.key());
        self.begin_iommu(peers, PARTY_IOMMU, iommu::PUBLISH, |back| {
            iommu::Wire::Publish(iommu::Step {
                op: iommu::PUBLISH,
                engine,
                key,
                hub,
                serial,
                reason: 0,
                back,
            })
        })
    }

    fn begin_enable(&mut self, peers: &Peers) -> bool {
        let Some(op) = self.operation.as_ref() else { return false; };
        let (serial, requester) = (op.serial, self.profile.requester);
        self.begin_pci(peers, PARTY_PCI, pci::ENABLE, |back| {
            pci::Wire::Enable(pci::Toggle {
                op: pci::ENABLE,
                serial,
                requester: u32::from(requester),
                back,
            })
        })
    }

    fn handoff(&mut self) -> bool {
        if self.operation.as_ref().is_some_and(|op| op.handoff.is_none()) {
            let op = self.operation.as_ref().unwrap();
            match ipc::session::establish::give(op.execution, dma::coord_mark(op.serial)) {
                Ok(writer) => self.operation.as_mut().unwrap().handoff = Some((
                    writer, ipc::time::Deadline::new(Wait::AtMost(MS)),
                )),
                Err(_) => {
                    self.fail(PARTY_RECOVERY, pci::ENABLE, 0);
                    return false;
                }
            }
        }
        let Some(op) = self.operation.as_ref() else {
            return false;
        };
        if op.stage == Stage::Active {
            return false;
        }
        let bar = match self.executor.bar_grant(op.serial) {
            Ok(bar) => bar,
            Err(error) => {
                self.fail(PARTY_RECOVERY, pci::ENABLE, api::fail_to_code(Some(error)));
                return false;
            }
        };
        let coords = Coords {
            bar_pages: op.pages,
            common: op.caps.0,
            notify: op.caps.1,
            multiplier: op.caps.2,
            device: op.caps.3,
            iova: op.iova,
            bytes: op.bytes,
            bar,
        };
        let mut buffer = [0u8; Coords::LEN];
        let Some(len) = coords.store(&mut buffer) else {
            self.fail(PARTY_RECOVERY, pci::ENABLE, 0);
            return false;
        };
        let (writer, deadline) = op.handoff.expect("retained handoff writer");
        match Hole::from_raw(writer).push(&buffer[..len], Wait::POLL) {
            Ok(()) => {}
            Err(error) if error.source.is_busy() && deadline.remaining() != Wait::POLL => return false,
            Err(_) => {
                self.fail(PARTY_RECOVERY, pci::ENABLE, 0);
                return false;
            }
        }
        let (serial, iova, execution) = (op.serial, op.iova, op.execution);
        self.stage(Stage::Active);
        crate::debug::put(&alloc::format!(
            "virtio-blk: binding {} active iova={:#x} execution={}",
            serial,
            iova,
            execution.get(),
        ));
        true
    }

    fn begin_invalidate(&mut self, peers: &Peers) -> bool {
        self.begin_step(peers, iommu::INVALIDATE, 0)
    }

    fn begin_disable(&mut self, peers: &Peers) -> bool {
        let Some(op) = self.operation.as_ref() else { return false; };
        let (serial, requester) = (op.serial, self.profile.requester);
        self.begin_pci(peers, PARTY_PCI, pci::DISABLE, |back| {
            pci::Wire::Disable(pci::Toggle {
                op: pci::DISABLE,
                serial,
                requester: u32::from(requester),
                back,
            })
        })
    }

    fn stop(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.executor.stop(serial) {
            Ok(Some(_)) => {
                self.stage(Stage::Quiesce);
                true
            }
            Ok(None) => false,
            Err(fail) => {
                self.fail(PARTY_RECOVERY, api::STOP, api::fail_to_code(Some(fail)));
                false
            }
        }
    }

    fn begin_quiesce(&mut self, peers: &Peers) -> bool {
        self.begin_step(peers, iommu::QUIESCE, 0)
    }

    fn begin_iommu_release(&mut self, peers: &Peers) -> bool {
        let reason = self.operation.as_ref().map(|op| op.reason.code()).unwrap_or(0);
        self.begin_step(peers, iommu::RELEASE, reason)
    }

    fn begin_step(&mut self, peers: &Peers, operation: u8, reason: u8) -> bool {
        let Some(op) = self.operation.as_ref() else { return false; };
        let (hub, serial) = (op.hub, op.serial);
        let (engine, key) = (self.domain.engine(), self.domain.key());
        self.begin_iommu(peers, PARTY_IOMMU, operation, |back| {
            let step = iommu::Step {
                op: operation,
                engine,
                key,
                hub,
                serial,
                reason,
                back,
            };
            match operation {
                iommu::INVALIDATE => iommu::Wire::Invalidate(step),
                iommu::QUIESCE => iommu::Wire::Quiesce(step),
                iommu::RELEASE => iommu::Wire::Release(step),
                _ => iommu::Wire::Invalidate(step),
            }
        })
    }

    fn detach(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.executor.detach(serial) {
            Ok(()) => {
                self.stage(Stage::CloseHandoff);
                true
            }
            Err(fail) => {
                self.fail(PARTY_RECOVERY, api::DETACH, api::fail_to_code(Some(fail)));
                false
            }
        }
    }

    fn close_handoff(&mut self) -> bool {
        if let Some((entry, _)) = self.operation.as_ref().and_then(|op| op.client) {
            if env::pie::release(entry, env::ReleaseMode::Revoke).is_err() {
                self.fail(PARTY_RECOVERY, api::DETACH, 0);
                return false;
            }
            self.operation.as_mut().unwrap().client = None;
        }
        if let Some((writer, _)) = self.operation.as_ref().and_then(|op| op.handoff) {
            if env::pie::release(writer, env::ReleaseMode::Revoke).is_err() {
                self.fail(PARTY_RECOVERY, api::DETACH, 0);
                return false;
            }
        }
        let op = self.operation.as_mut().unwrap();
        op.handoff = None;
        op.stage = Stage::Unplace;
        true
    }

    fn begin_unplace(&mut self, peers: &Peers) -> bool {
        let Some(op) = self.operation.as_ref() else { return false; };
        let (serial, requester) = (op.serial, self.profile.requester);
        self.begin_pci(peers, PARTY_PCI, pci::RELEASE, |back| {
            pci::Wire::Release(pci::Toggle {
                op: pci::RELEASE,
                serial,
                requester: u32::from(requester),
                back,
            })
        })
    }

    fn complete_pci(&mut self, stage: Stage, said: pci::Said) -> bool {
        match stage {
            Stage::Place => {
                if said.status == wire::OK {
                    if let Some(op) = self.operation.as_mut() {
                        op.bar = Some(said.bar);
                        op.pages = said.pages;
                    }
                    self.advance_after_prepare(Stage::Attach);
                    true
                } else {
                    self.fail(PARTY_PCI, pci::PLACE, said.status);
                    self.reconcile_close();
                    false
                }
            }
            Stage::Enable => {
                if said.status == wire::OK {
                    self.advance_after_prepare(Stage::Handoff);
                    true
                } else {
                    self.fail(PARTY_PCI, pci::ENABLE, said.status);
                    self.reconcile_close();
                    false
                }
            }
            Stage::Disable => {
                if said.status == wire::OK {
                    self.stage(Stage::Stop);
                    true
                } else {
                    self.fail(PARTY_PCI, pci::DISABLE, said.status);
                    false
                }
            }
            Stage::Unplace => {
                if said.status == wire::OK {
                    self.done();
                    true
                } else {
                    self.fail(PARTY_PCI, pci::RELEASE, said.status);
                    false
                }
            }
            _ => false,
        }
    }

    fn complete_iommu(&mut self, stage: Stage, said: iommu::Said) -> bool {
        match stage {
            Stage::Acquire => {
                if said.status == wire::OK
                    && said.engine == self.domain.engine()
                    && said.key == self.domain.key()
                    && said.iova != 0
                    && said.bytes != 0
                {
                    if let Some(op) = self.operation.as_mut() {
                        op.iova = said.iova;
                        op.bytes = said.bytes;
                    }
                    self.advance_after_prepare(Stage::Publish);
                    true
                } else if said.status == iommu::fail_to_code(Some(iommu::Fail::NotReady)) {
                    false
                } else {
                    self.fail(PARTY_IOMMU, iommu::ACQUIRE, said.status);
                    self.reconcile_close();
                    false
                }
            }
            Stage::Publish => {
                if said.status == wire::OK && said.pending == 0 {
                    self.advance_after_prepare(Stage::Enable);
                    true
                } else if said.status == wire::OK {
                    false
                } else {
                    self.fail(PARTY_IOMMU, iommu::PUBLISH, said.status);
                    self.reconcile_close();
                    false
                }
            }
            Stage::Invalidate => self.complete_step(said, iommu::INVALIDATE, Stage::Disable),
            Stage::Quiesce => self.complete_step(said, iommu::QUIESCE, Stage::Release),
            Stage::Release => self.complete_step(said, iommu::RELEASE, Stage::Detach),
            _ => false,
        }
    }

    fn complete_step(&mut self, said: iommu::Said, operation: u8, next: Stage) -> bool {
        if said.status != wire::OK {
            self.fail(PARTY_IOMMU, operation, said.status);
            return false;
        }
        if said.pending != 0 {
            return false;
        }
        self.stage(next);
        true
    }

    fn advance_after_prepare(&mut self, next: Stage) {
        let Some(op) = self.operation.as_mut() else { return; };
        op.stage = if op.closing { Stage::Invalidate } else { next };
    }

    fn reconcile_close(&mut self) {
        let Some(op) = self.operation.as_mut() else { return; };
        if op.closing && !is_cleanup(op.stage) {
            op.stage = Stage::Invalidate;
        }
    }

    fn done(&mut self) {
        if let Some(op) = self.operation.as_mut() {
            op.stage = Stage::Done;
            op.closing = false;
            self.last_serial = op.serial;
            crate::debug::put(&alloc::format!(
                "virtio-blk: binding {} closed by coordinator",
                op.serial
            ));
        }
    }

    fn stage(&mut self, stage: Stage) {
        if let Some(op) = self.operation.as_mut() {
            op.stage = stage;
        }
    }

    fn fail(&mut self, party: u8, op: u8, code: u8) {
        if let Some(operation) = self.operation.as_mut() {
            if operation.first_failure.is_none() {
                crate::debug::put(&alloc::format!(
                    "virtio-blk: binding {} stage {:?} failed party={} op={} code={}",
                    operation.serial,
                    operation.stage,
                    party,
                    op,
                    code,
                ));
            }
            operation.fail(party, op, code);
        }
    }

    fn reply(&self) -> DmaSaid {
        match self.operation.as_ref() {
            Some(op) => {
                let mut said = DmaSaid::progress(op.serial, op.state(), op.first_failure);
                said.client = op.client.map(|(_, remote)| remote).unwrap_or(env::PieToken::NONE);
                said
            }
            None => DmaSaid::of(fail_to_code(Some(Fail::Unknown))),
        }
    }
}

fn is_cleanup(stage: Stage) -> bool {
    matches!(
        stage,
        Stage::Invalidate
            | Stage::Disable
            | Stage::Stop
            | Stage::Quiesce
            | Stage::Release
            | Stage::Detach
            | Stage::CloseHandoff
            | Stage::Unplace
    )
}

fn pci_op(stage: Stage) -> u8 {
    match stage {
        Stage::Place => pci::PLACE,
        Stage::Enable => pci::ENABLE,
        Stage::Disable => pci::DISABLE,
        Stage::Unplace => pci::RELEASE,
        _ => 0,
    }
}

fn iommu_op(stage: Stage) -> u8 {
    match stage {
        Stage::Acquire => iommu::ACQUIRE,
        Stage::Publish => iommu::PUBLISH,
        Stage::Invalidate => iommu::INVALIDATE,
        Stage::Quiesce => iommu::QUIESCE,
        Stage::Release => iommu::RELEASE,
        _ => 0,
    }
}
