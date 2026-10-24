//! Concrete coordination cursor owned by the independent Recovery Driver.

use env::{TaskId, Wait};
use hub_api::coordination::{Prepare, Release, Wire};
use hub_api::{
    fail_to_code, Close, DmaSaid, Fail, PARTY_IOMMU,
    PARTY_PCI, PARTY_RECOVERY,
};
use resource::raw::Hole;
use wire::Message;

use crate::driver::iommu::api as iommu;
use crate::driver::pci::api as pci;
use crate::driver::recovery::api as recovery;
use crate::driver::recovery::core::{AttachRequest, Core};
use crate::driver::recovery::operation::Operation;
use crate::driver::recovery::profile::Profile;
use crate::driver::recovery::stage::Stage;
use crate::driver::shared::dma::{self, Coords};
use crate::driver::shared::peer::Peer;

const MS: usize = 3000;

pub struct Coordinator {
    pub hub: TaskId,
    pub pci: Peer<pci::Call>,
    pub iommu: Peer<iommu::Call>,
    pub core: Core,
    pub operation: Option<Operation>,
    pub last_serial: u64,
    pub profile: Profile,
}

impl Coordinator {
    pub fn new(
        hub: TaskId,
        pci: Peer<pci::Call>,
        iommu: Peer<iommu::Call>,
        core: Core,
        profile: Profile,
    ) -> Self {
        Self {
            hub,
            pci,
            iommu,
            core,
            operation: None,
            last_serial: 0,
            profile,
        }
    }

    pub fn turn(&mut self, from: TaskId, wire: Wire) -> DmaSaid {
        match wire {
            Wire::Prepare(frame) => {
                if from != self.hub || frame.hub != from {
                    return DmaSaid::of(fail_to_code(Some(Fail::Denied)));
                }
                if let Err(fail) = self.begin(frame) {
                    return DmaSaid::of(fail_to_code(Some(fail)));
                }
                self.prepare();
                self.reply()
            }
            Wire::Release(frame) => {
                if from != self.hub || frame.hub != from {
                    return DmaSaid::of(fail_to_code(Some(Fail::Denied)));
                }
                if let Err(fail) = self.begin_release(frame) {
                    return DmaSaid::of(fail_to_code(Some(fail)));
                }
                self.release();
                self.reply()
            }
        }
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
        self.operation.as_mut().unwrap().bar = retained_bar;
        self.operation.as_mut().unwrap().caps = self.profile.caps;
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
        if matches!(
            op.stage,
            Stage::Active
                | Stage::Handoff
                | Stage::Enable
                | Stage::Publish
                | Stage::Acquire
                | Stage::Attach
                | Stage::Place
        ) {
            op.stage = Stage::Invalidate;
        }
        Ok(())
    }

    fn prepare(&mut self) {
        loop {
            let Some(stage) = self.operation.as_ref().map(|op| op.stage) else {
                return;
            };
            let advanced = match stage {
                Stage::Place => self.place(),
                Stage::Attach => self.attach(),
                Stage::Acquire => self.acquire(),
                Stage::Publish => self.publish(),
                Stage::Enable => self.enable(),
                Stage::Handoff => self.handoff(),
                _ => false,
            };
            if !advanced {
                return;
            }
        }
    }

    fn release(&mut self) {
        let Some(stage) = self.operation.as_ref().map(|op| op.stage) else { return; };
        match stage {
            Stage::Invalidate => self.invalidate(),
            Stage::Disable => self.disable(),
            Stage::Stop => self.stop(),
            Stage::Quiesce => self.quiesce(),
            Stage::Release => self.iommu_release(),
            Stage::Detach => self.detach(),
            Stage::CloseHandoff => self.close_handoff(),
            Stage::Unplace => self.unplace(),
            _ => false,
        };
    }

    fn place(&mut self) -> bool {
        let Some(op) = self.operation.as_ref() else {
            return false;
        };
        let requester = self.profile.requester;
        let serial = op.serial;
        match self.pci.call(Wait::AtMost(MS), |back| {
            pci::Wire::Place(pci::Place {
                op: pci::PLACE,
                serial,
                requester: requester as u32,
                bar: self.profile.bar,
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK => {
                if let Some(op) = self.operation.as_mut() {
                    // Retain the delivered BAR and geometry before fallible attach.
                    op.bar = Some(said.bar);
                    op.pages = said.pages;
                    op.stage = Stage::Attach;
                }
                true
            }
            Ok(said) => {
                self.fail(PARTY_PCI, pci::PLACE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_PCI, pci::PLACE, 0);
                false
            }
        }
    }

    fn attach(&mut self) -> bool {
        let Some(op) = self.operation.as_ref() else {
            return false;
        };
        let Some(bar) = op.bar else {
            self.fail(PARTY_RECOVERY, recovery::ATTACH, 0);
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
        match self.core.attach(request) {
            Ok(()) => {
                if let Some(op) = self.operation.as_mut() {
                    op.stage = Stage::Acquire;
                }
                true
            }
            Err(fail) => {
                self.fail(PARTY_RECOVERY, recovery::ATTACH, recovery::fail_to_code(Some(fail)));
                false
            }
        }
    }

    fn acquire(&mut self) -> bool {
        let Some(op) = self.operation.as_ref() else {
            return false;
        };
        let serial = op.serial;
        let requester = u32::from(self.profile.requester);
        let execution = op.execution;
        let iova = match hub_api::iova_base(serial) {
            Some(iova) => iova,
            None => {
                self.fail(PARTY_RECOVERY, iommu::ACQUIRE, 0);
                return false;
            }
        };
        let access = env::Access::FETCH_STORE.bits().bits();
        match self.iommu.call(Wait::AtMost(MS), |back| {
            iommu::Wire::Acquire(iommu::Acquire {
                op: iommu::ACQUIRE,
                serial,
                requester,
                execution,
                iova,
                access,
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK => {
                if let Some(op) = self.operation.as_mut() {
                    op.iova = said.iova;
                    op.stage = Stage::Publish;
                }
                true
            }
            Ok(said) if said.status == iommu::fail_to_code(Some(iommu::Fail::NotReady)) => false,
            Ok(said) => {
                self.fail(PARTY_IOMMU, iommu::ACQUIRE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_IOMMU, iommu::ACQUIRE, 0);
                false
            }
        }
    }

    fn publish(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.iommu.call(Wait::AtMost(MS), |back| {
            iommu::Wire::Publish(iommu::Step {
                op: iommu::PUBLISH,
                serial,
                reason: 0,
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK && said.pending == 0 => {
                self.stage(Stage::Enable);
                true
            }
            Ok(said) if said.status == wire::OK => false,
            Ok(said) => {
                self.fail(PARTY_IOMMU, iommu::PUBLISH, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_IOMMU, iommu::PUBLISH, 0);
                false
            }
        }
    }

    fn enable(&mut self) -> bool {
        let Some(op) = self.operation.as_ref() else {
            return false;
        };
        let serial = op.serial;
        match self.pci.call(Wait::AtMost(MS), |back| {
            pci::Wire::Enable(pci::Toggle {
                op: pci::ENABLE,
                serial,
                requester: u32::from(self.profile.requester),
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK => {
                if let Some(op) = self.operation.as_mut() {
                    op.stage = Stage::Handoff;
                }
                true
            }
            Ok(said) => {
                self.fail(PARTY_PCI, pci::ENABLE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_PCI, pci::ENABLE, 0);
                false
            }
        }
    }

    fn handoff(&mut self) -> bool {
        if self.operation.as_ref().is_some_and(|op| op.handoff.is_none()) {
            let op = self.operation.as_ref().unwrap();
            match ipc::session::establish::give(op.execution, dma::coord_mark(op.serial)) {
                Ok(writer) => self.operation.as_mut().unwrap().handoff = Some(writer),
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
        let coords = Coords {
            pages: op.pages,
            common: op.caps.0,
            notify: op.caps.1,
            multiplier: op.caps.2,
            device: op.caps.3,
            iova: op.iova,
        };
        let mut buffer = [0u8; Coords::LEN];
        let Some(len) = coords.store(&mut buffer) else {
            self.fail(PARTY_RECOVERY, pci::ENABLE, 0);
            return false;
        };
        let writer = op.handoff.expect("retained handoff writer");
        if Hole::from_raw(writer)
            .push(&buffer[..len], Wait::AtMost(MS))
            .is_err()
        {
            self.fail(PARTY_RECOVERY, pci::ENABLE, 0);
            return false;
        }
        let (serial, iova, execution) = (op.serial, op.iova, op.execution);
        self.stage(Stage::Active);
        crate::debug::put(&alloc::format!(
            "recovery: binding {} active iova={:#x} execution={}",
            serial,
            iova,
            execution.get(),
        ));
        true
    }

    fn invalidate(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.iommu.call(Wait::AtMost(MS), |back| {
            iommu::Wire::Invalidate(iommu::Step {
                op: iommu::INVALIDATE,
                serial,
                reason: 0,
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK && said.pending == 0 => {
                self.stage(Stage::Disable);
                true
            }
            Ok(said) if said.status == wire::OK => false,
            Ok(said) => {
                self.fail(PARTY_IOMMU, iommu::INVALIDATE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_IOMMU, iommu::INVALIDATE, 0);
                false
            }
        }
    }

    fn disable(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.pci.call(Wait::AtMost(MS), |back| {
            pci::Wire::Disable(pci::Toggle {
                op: pci::DISABLE,
                serial,
                requester: u32::from(self.profile.requester),
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK => {
                self.stage(Stage::Stop);
                true
            }
            Ok(said) => {
                self.fail(PARTY_PCI, pci::DISABLE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_PCI, pci::DISABLE, 0);
                false
            }
        }
    }

    fn stop(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.core.stop(serial) {
            Ok(_) => {
                self.stage(Stage::Quiesce);
                true
            }
            Err(fail) => {
                self.fail(PARTY_RECOVERY, recovery::STOP, recovery::fail_to_code(Some(fail)));
                false
            }
        }
    }

    fn quiesce(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.iommu.call(Wait::AtMost(MS), |back| {
            iommu::Wire::Quiesce(iommu::Step {
                op: iommu::QUIESCE,
                serial,
                reason: 0,
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK && said.pending == 0 => {
                self.stage(Stage::Release);
                true
            }
            Ok(said) if said.status == wire::OK => false,
            Ok(said) => {
                self.fail(PARTY_IOMMU, iommu::QUIESCE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_IOMMU, iommu::QUIESCE, 0);
                false
            }
        }
    }

    fn iommu_release(&mut self) -> bool {
        let Some(op) = self.operation.as_ref() else {
            return false;
        };
        let (serial, reason) = (op.serial, op.reason.code());
        match self.iommu.call(Wait::AtMost(MS), |back| {
            iommu::Wire::Release(iommu::Step {
                op: iommu::RELEASE,
                serial,
                reason,
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK && said.pending == 0 => {
                if let Some(op) = self.operation.as_mut() {
                    op.stage = Stage::Detach;
                }
                true
            }
            Ok(said) if said.status == wire::OK => false,
            Ok(said) => {
                self.fail(PARTY_IOMMU, iommu::RELEASE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_IOMMU, iommu::RELEASE, 0);
                false
            }
        }
    }

    fn detach(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.core.detach(serial) {
            Ok(()) => {
                if let Some(op) = self.operation.as_mut() {
                    op.stage = Stage::CloseHandoff;
                }
                true
            }
            Err(fail) => {
                self.fail(PARTY_RECOVERY, recovery::DETACH, recovery::fail_to_code(Some(fail)));
                false
            }
        }
    }

    fn unplace(&mut self) -> bool {
        let Some(serial) = self.operation.as_ref().map(|op| op.serial) else {
            return false;
        };
        match self.pci.call(Wait::AtMost(MS), |back| {
            pci::Wire::Release(pci::Toggle {
                op: pci::RELEASE,
                serial,
                requester: u32::from(self.profile.requester),
                back,
            })
        }) {
            Ok(said) if said.status == wire::OK => {
                self.done();
                true
            }
            Ok(said) => {
                self.fail(PARTY_PCI, pci::RELEASE, said.status);
                false
            }
            Err(_) => {
                self.fail(PARTY_PCI, pci::RELEASE, 0);
                false
            }
        }
    }

    fn close_handoff(&mut self) -> bool {
        if let Some(writer) = self.operation.as_ref().and_then(|op| op.handoff) {
            if env::pie::release(writer, env::ReleaseMode::Revoke).is_err() {
                self.fail(PARTY_RECOVERY, recovery::DETACH, 0);
                return false;
            }
        }
        let op = self.operation.as_mut().unwrap();
        op.handoff = None;
        op.stage = Stage::Unplace;
        true
    }

    fn done(&mut self) {
        if let Some(op) = self.operation.as_mut() {
            op.stage = Stage::Done;
            self.last_serial = op.serial;
            crate::debug::put(&alloc::format!(
                "recovery: binding {} closed by coordinator",
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
                    "recovery: binding {} stage {:?} failed party={} op={} code={}",
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
            Some(op) => DmaSaid::progress(op.serial, op.state(), op.first_failure),
            None => DmaSaid::of(fail_to_code(Some(Fail::Unknown))),
        }
    }
}
