use alloc::vec::Vec;
use env::{Mark, PieToken, TaskId, Wait};
use ipc::session::{Endpoint, establish};

use super::{start::Error, table::Table, task::Readiness, verdict::Fail};

pub struct Service {
    task: TaskId,
    channels: Vec<Endpoint>,
    supplies: Vec<PieToken>,
}
impl Service {
    pub(super) fn new(task: TaskId) -> Self {
        Self {
            task,
            channels: Vec::new(),
            supplies: Vec::new(),
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
        for supply in program.supply() {
            for channel in [Some(supply.channel()), supply.ready()]
                .into_iter()
                .flatten()
            {
                staged
                    .channels
                    .try_reserve(1)
                    .map_err(|_| Error::Step("no room for channels"))?;
                staged.channels.push(
                    establish::endpoint(self.task, Mark::of(channel), Wait::POLL)
                        .map_err(|_| Error::Step("connect failed"))?,
                );
            }
        }
        core::mem::swap(&mut self.channels, &mut staged.channels);
        Ok(())
    }
    pub fn ready(&mut self, table: &mut Table, readiness: Readiness<'_>) -> Result<bool, Fail> {
        let name = readiness.name;
        let result = super::task::ready(table, readiness, &mut self.channels);
        if table
            .find(name)
            .is_some_and(|row| row.state == super::table::State::Ready)
        {
            self.clear_supplies();
        }
        result
    }
    pub(crate) fn hold_supply(&mut self, token: PieToken) -> Result<(), Error> {
        if self.supplies.try_reserve(1).is_err() {
            let _ = env::pie::shut(token);
            let _ = env::pie::release(token, env::ReleaseMode::Revoke);
            return Err(Error::Step("no room for image supplies"));
        }
        self.supplies.push(token);
        Ok(())
    }
    fn clear_supplies(&mut self) {
        for token in self.supplies.drain(..) {
            let _ = env::pie::shut(token);
            let _ = env::pie::release(token, env::ReleaseMode::Revoke);
        }
    }
    pub(crate) fn claim_supply(&mut self, mark: Mark, wait: Wait) -> Result<PieToken, Error> {
        let link = self
            .channels
            .iter_mut()
            .find(|channel| {
                resource::raw::reserve(channel.rx()).is_ok_and(|(_, _, role)| role == mark)
            })
            .ok_or(Error::Step("no channel"))?;
        match link.claim(self.task, mark, wait) {
            Ok(true) => link.tx().ok_or(Error::Step("no channel")),
            Err(establish::DiscoveryFail::Ambiguous) => Err(Error::Step("ambiguous channel")),
            _ => Err(Error::Step("no channel")),
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.clear_supplies();
        for channel in &self.channels {
            let _ = env::pie::release(channel.rx(), env::ReleaseMode::Revoke);
        }
    }
}
