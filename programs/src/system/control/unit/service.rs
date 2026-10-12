use alloc::vec::Vec;
use env::{Mark, PieToken, TaskId, Wait};
use ipc::session::{Endpoint, establish};

use super::{
    start::{BOOT_MS, Error},
    table::Table,
    task::Readiness,
    verdict::Fail,
};

pub struct Service {
    task: TaskId,
    channels: Vec<Endpoint>,
    descriptors: Vec<DescriptorChannel>,
}

#[derive(Clone, Copy)]
struct DescriptorChannel {
    slot: Mark,
    writer: PieToken,
}

impl Service {
    pub(crate) fn new(task: TaskId) -> Self {
        Self {
            task,
            channels: Vec::new(),
            descriptors: Vec::new(),
        }
    }
    pub fn task(&self) -> TaskId {
        self.task
    }
    pub fn connect(&mut self, program: &crate::unit::UnitFile) -> Result<(), Error> {
        if !program.valid() {
            return Err(Error::Step("invalid supply declaration"));
        }
        let mut staged = Self::new(self.task);
        for binding in program.supply() {
            staged
                .descriptors
                .try_reserve(1)
                .map_err(|_| Error::Step("no room for descriptors"))?;
            let writer = establish::give(self.task, crate::unit::descriptor_mark(binding.slot))
                .map_err(|_| Error::Step("descriptor channel"))?;
            staged.descriptors.push(DescriptorChannel {
                slot: binding.slot,
                writer,
            });
        }
        if let crate::unit::Ready::Signal(mark) = program.control.ready {
            staged
                .channels
                .try_reserve(1)
                .map_err(|_| Error::Step("no room for ready channel"))?;
            staged.channels.push(
                establish::endpoint(self.task, mark, Wait::POLL)
                    .map_err(|_| Error::Step("ready channel"))?,
            );
        }
        core::mem::swap(&mut self.channels, &mut staged.channels);
        core::mem::swap(&mut self.descriptors, &mut staged.descriptors);
        Ok(())
    }
    pub fn ready(&mut self, table: &mut Table, readiness: Readiness<'_>) -> Result<bool, Fail> {
        let result = super::task::ready(table, readiness, &mut self.channels);
        result
    }
    pub(crate) fn ready_for(&mut self, ready: crate::unit::Ready) -> Result<bool, Fail> {
        match ready {
            crate::unit::Ready::OnSpawn => {
                if env::unit::join_task(self.task, Wait::POLL).unwrap_or(true) {
                    Err(Fail::NotReady)
                } else {
                    Ok(true)
                }
            }
            crate::unit::Ready::Signal(mark) => {
                let channel = self.channels.first_mut().ok_or(Fail::NotReady)?;
                match channel.claim(self.task, mark, Wait::POLL) {
                    Ok(true) => Ok(true),
                    Ok(false) if !env::unit::join_task(self.task, Wait::POLL).unwrap_or(true) => {
                        Ok(false)
                    }
                    Ok(false) => Err(Fail::NotReady),
                    Err(ipc::session::establish::DiscoveryFail::Ambiguous) => Err(Fail::Unknown),
                    Err(ipc::session::establish::DiscoveryFail::Missing) => Err(Fail::NotReady),
                }
            }
        }
    }
    pub(crate) fn send_descriptor(&self, slot: Mark, bytes: &[u8]) -> Result<(), Error> {
        let channel = self
            .descriptors
            .iter()
            .find(|channel| channel.slot == slot)
            .ok_or(Error::Step("supply descriptor channel missing"))?;
        resource::raw::Hole::from_raw(channel.writer)
            .push(bytes, Wait::AtMost(BOOT_MS))
            .map_err(|_| Error::Step("supply descriptor send"))
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        for channel in &self.channels {
            let _ = env::pie::release(channel.rx(), env::ReleaseMode::Revoke);
        }
        for channel in &self.descriptors {
            let _ = env::pie::release(channel.writer, env::ReleaseMode::Revoke);
        }
    }
}
