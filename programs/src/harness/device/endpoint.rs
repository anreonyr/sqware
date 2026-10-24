use env::{AcquireSource, PAGE_SIZE, PieToken, Placement, TaskId, TeamId, ViewControl};
use execution::memory::{Reference, View};
use ipc::session::establish;

use crate::driver::shared::dma::{self, HubDma};
use super::{control::{self, Command, Report}, layout::{self, Block}};

const MS: usize = 3000;

/// One physical device's execution and the controller-owned DMA window.
pub(super) struct Endpoint {
    pole: PieToken,
    reference: Reference,
    view: View,
    writer: PieToken,
    pub task: TaskId,
    pub report: Report,
    pub iova: u64,
    pattern: u8,
}

impl Endpoint {
    pub fn open(
        sire: TaskId, hub: &HubDma, launch: PieToken, entry: PieToken, id: u64, pattern: u8,
    ) -> Result<Self, &'static str> {
        let pole = resource::raw::pole(layout::SPAN as usize, true).map_err(|_| "dual: pages")?;
        let reference = Reference::acquire(AcquireSource::Source {
            token: pole, offset: 0, count: layout::PAGES,
        }, PieToken::NONE).map_err(|_| "dual: reference")?;
        let view = reference.attach(
            TeamId::new(0), 0, layout::PAGES,
            Placement::lowest(PAGE_SIZE).ok_or("dual: placement")?, 6, ViewControl::Caller,
        ).map_err(|_| "dual: view")?;
        let (task, writer, report, iova) = Self::start(
            sire, hub, launch, entry, id, pole, view.address(),
        )?;
        Ok(Self { pole, reference, view, writer, task, report, iova, pattern })
    }

    fn start(
        sire: TaskId, hub: &HubDma, launch: PieToken, entry: PieToken, id: u64,
        pole: PieToken, address: usize,
    ) -> Result<(TaskId, PieToken, Report, u64), &'static str> {
        let task = control::construct(sire, hub.host(), launch, "device-work", id)?;
        let writer = establish::give(task, control::COMMAND).map_err(|_| "dual: command")?;
        let report = control::receive_report(entry, task)?;
        control::check_auxiliary(report, true)?;
        if report.want != 1 || report.serial == 0 { return Err("dual: claim report"); }
        let block = Block::at(address);
        for page in 0..layout::PAGES { block.fill(page, 0); }
        let window = env::pie::accord(
            pole, task, env::Permission::FETCH | env::Permission::STORE | env::Permission::VEST,
            dma::window_mark(report.serial),
        ).map_err(|_| "dual: window handover")?;
        control::command(writer, Command::Window(window))?;
        let active = control::activation(entry, task, report.serial)?;
        Ok((task, writer, report, active.iova))
    }

    pub fn address(&self) -> usize { self.view.address() }

    pub fn read(&self, round: u16) -> Result<(), &'static str> {
        let block = Block::at(self.view.address());
        block.fill(layout::DATA, layout::SENTINEL);
        block.put32(layout::STATUS, 0, 0xff);
        block.arm(self.iova, round);
        control::command(self.writer, Command::Ring)?;
        let until = env::chrono::clock().saturating_add(MS as u64 * 1_000_000);
        while block.used_index() < round + 1 {
            if env::chrono::clock() >= until { return Err("dual: read timeout"); }
            let _ = execution::room::park(::core::time::Duration::from_millis(1));
        }
        self.verify(round)?;
        let at = 4 + usize::from(round) * 8;
        let len = block.get32(layout::USED, at + 4);
        crate::debug::put(&alloc::format!(
            "dual: read serial={} pattern={} round={round} used={} len={len} sector_ok=true status=0",
            self.report.serial, self.pattern, block.used_index(),
        ));
        Ok(())
    }

    pub fn verify(&self, round: u16) -> Result<(), &'static str> {
        let block = Block::at(self.view.address());
        let at = 4 + usize::from(round) * 8;
        if !(0..layout::SECTOR).all(|i| block.data(i) == (layout::expected(i) ^ self.pattern))
            || block.status() != layout::BLK_OK || block.used_index() != round + 1
            || block.get32(layout::USED, at) != 0
            || block.get32(layout::USED, at + 4) != layout::WRITTEN_BYTES
        { return Err("dual: wrong DMA destination or sector"); }
        Ok(())
    }

    pub fn close(&self, hub: &HubDma, fault: bool) -> Result<(), &'static str> {
        control::command(self.writer, if fault { Command::Exit } else { Command::Close })?;
        control::await_closed(hub, self.report.serial, MS)?;
        control::check_auxiliary(self.report, false)
    }

    pub fn rebind(
        &mut self, sire: TaskId, hub: &HubDma, launch: PieToken, entry: PieToken, id: u64,
    ) -> Result<(), &'static str> {
        let (task, writer, report, iova) = Self::start(
            sire, hub, launch, entry, id, self.pole, self.view.address(),
        )?;
        if report.serial <= self.report.serial || iova == self.iova {
            return Err("dual: binding or DMA range reused");
        }
        resource::raw::release(self.writer).map_err(|_| "dual: old command release")?;
        self.task = task;
        self.writer = writer;
        self.report = report;
        self.iova = iova;
        Ok(())
    }

    pub fn release(&mut self) -> Result<(), &'static str> {
        resource::raw::release(self.writer).map_err(|_| "dual: command release")?;
        self.view.close().map_err(|_| "dual: view close")?;
        self.reference.release().map_err(|_| "dual: reference release")?;
        resource::raw::release(self.pole).map_err(|_| "dual: pole release")
    }
}
