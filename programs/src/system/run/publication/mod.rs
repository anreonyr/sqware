use crate::system::operator::Placement;
use env::{PieToken, TaskId};
use system_api::operator::PathBuf;
use system_api::control::publication::Frame;
use system_api::control::publication::Reply;
use system_api::control::publication::Target;
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Permit;
pub mod install;
pub mod identity;
pub mod operator;
pub mod internal;
pub mod policy;
pub mod receive;
pub mod retire;
pub struct Internal {
    pub road: PathBuf,
    pub entry: PieToken,
    pub access: (Permit, TaskId),
}
pub struct Incoming {
    pub frame: Frame,
    pub from: TaskId,
    pub back: Option<ipc::rpc::reply::Sender<Reply>>,
}
pub struct Inbox(pub alloc::collections::VecDeque<Incoming>);
pub struct Request(pub Option<Incoming>);
pub struct Outcome(pub Option<Result<Reply, Fail>>);
pub struct Approved {
    pub target: Target,
    pub placement: Placement,
    pub publisher: TaskId,
}
pub enum Decision {
    Unset,
    Device,
    OwnHole(Approved),
    Install(Approved),
    Mounted(Approved, EntryId),
    Remove(usize),
    Removed,
    Failed(Fail),
}
pub struct Kind {
    pub road: Option<PathBuf>,
}

pub mod faces;

pub mod book;
