use alloc::vec::Vec;
use env::{PieToken, TaskId};
use system_api::operator::path::PathBuf;
use system_api::control::Target;
use system_client::operator::{EntryId, Permit};

pub(crate) struct Address {
    pub(crate) road: PathBuf,
    pub(crate) target: Option<Target>,
}
pub(crate) struct Source {
    pub(crate) publisher: TaskId,
    pub(crate) entry: PieToken,
    pub(crate) permit: Permit,
}
pub(crate) struct Installation {
    pub(crate) owner: TaskId,
    pub(crate) mount: Option<EntryId>,
}
pub(crate) struct Record {
    pub(crate) address: Address,
    pub(crate) source: Source,
    pub(crate) installation: Installation,
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
        self.records.iter().any(|r| r.source.entry == entry)
    }
}
