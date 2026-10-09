use alloc::vec::Vec;
use pipe_api::{Direction, Fail};
pub const LIMIT: usize = 256;
#[derive(Clone)]
pub struct Record {
    pub id: u64,
    pub owner: usize,
    pub read: Option<usize>,
    pub write: Option<usize>,
}
#[derive(Default)]
pub struct Ledger {
    records: Vec<Record>,
    next: u64,
}
impl Ledger {
    pub fn create(&mut self, owner: usize, capacity: usize) -> Result<u64, Fail> {
        if owner == 0 || capacity == 0 || capacity > pipe_api::MAX_CAPACITY {
            return Err(Fail::Invalid);
        }
        if self.records.len() >= LIMIT
            || self.records.iter().filter(|r| r.owner == owner).count() >= 64
        {
            return Err(Fail::Full);
        }
        self.records.try_reserve(1).map_err(|_| Fail::Full)?;
        self.next = self.next.checked_add(1).ok_or(Fail::Full)?;
        self.records.push(Record {
            id: self.next,
            owner,
            read: None,
            write: None,
        });
        Ok(self.next)
    }
    pub fn record(&self, id: u64) -> Result<&Record, Fail> {
        self.records.iter().find(|r| r.id == id).ok_or(Fail::Dead)
    }
    pub fn authorize(&self, id: u64, owner: usize) -> Result<(), Fail> {
        if self.record(id)?.owner == owner {
            Ok(())
        } else {
            Err(Fail::Denied)
        }
    }
    pub fn target(&self, id: u64, direction: Direction) -> Result<Option<usize>, Fail> {
        let r = self.record(id)?;
        Ok(match direction {
            Direction::Read => r.read,
            Direction::Write => r.write,
        })
    }
    pub fn bind(
        &mut self,
        id: u64,
        owner: usize,
        direction: Direction,
        target: usize,
    ) -> Result<(), Fail> {
        self.authorize(id, owner)?;
        if target == 0 {
            return Err(Fail::Invalid);
        }
        let r = self
            .records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(Fail::Dead)?;
        let slot = match direction {
            Direction::Read => &mut r.read,
            Direction::Write => &mut r.write,
        };
        if slot.is_some() {
            return Err(Fail::Denied);
        }
        *slot = Some(target);
        Ok(())
    }
    pub fn release(&mut self, id: u64) {
        if let Some(index) = self.records.iter().position(|r| r.id == id) {
            self.records.swap_remove(index);
        }
    }
}
