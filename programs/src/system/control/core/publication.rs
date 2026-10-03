use alloc::vec::Vec;
use env::{PieToken, TaskId};
use protocol::common::path::PathBuf;
use protocol::system::control::publication::Target;
use protocol::system::operator::{EntryId, Permit};

pub(crate) struct Record {
    pub(crate) road: PathBuf,
    pub(crate) target: Option<Target>,
    pub(crate) publisher: TaskId,
    pub(crate) owner: TaskId,
    pub(crate) entry: PieToken,
    pub(crate) permit: Permit,
    pub(crate) mount: Option<EntryId>,
}

pub struct Publications {
    pub(crate) records: Vec<Record>,
}
impl Publications {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }
    pub(crate) fn owns(&self, entry: PieToken) -> bool {
        self.records.iter().any(|r| r.entry == entry)
    }
}
