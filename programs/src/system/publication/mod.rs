use crate::system::operator::Placement;
use env::{PieToken, TaskId};
use system_api::control::publication::Frame;
use system_api::control::publication::Reply;
use system_api::control::publication::Target;
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::PathBuf;
use system_api::operator::Permit;
extern crate alloc;
mod admission;
pub(crate) use admission::{Namespace, Namespaces, Permission};
mod identity;
mod install;
mod internal;
mod policy;
mod receive;
mod retire;
pub(crate) struct Entry(pub PieToken);
impl Entry {
    pub(crate) fn inject(&self, task: TaskId) -> Result<(), &'static str> {
        ::resource::port::ship(self.0, task, env::Access::STORE, env::Policy::NONE)
            .map(|_| ())
            .map_err(|_| "publication inject")
    }
}
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
pub struct Request(pub Option<Incoming>, pub Option<alloc::string::String>);
pub struct Outcome(pub Option<Result<Reply, Fail>>);
pub struct Approval {
    pub target: Target,
    pub member: bool,
    pub alias: bool,
}
pub struct Approved {
    pub policy: Approval,
    pub placement: Placement,
    pub publisher: TaskId,
}
pub enum Decision {
    Unset,
    BindAlias {
        publisher: TaskId,
        registration: names::Registration,
    },
    AliasMounted(EntryId),
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

mod faces;

mod book;

mod living;
mod names;
mod runtime;
pub(crate) struct Mounts(pub alloc::vec::Vec<Internal>);

mod resources;
pub(crate) use resources::install as register;

mod plan;
pub(crate) use plan::{maintenance, prepare_runtime, retire_tasks};

pub(crate) use book::Publications;
pub(crate) use faces::{faces as control_faces, instance_face, publication_face};
pub(crate) use identity::faces as identity_faces;
pub(crate) use names::{Names, Registration};
pub(crate) use runtime::{Approval as RuntimeApproval, Resources as RuntimeNamespace};
pub(crate) fn observed_revision(
    resources: &::schedule::Resources<'_>,
) -> Result<u64, &'static str> {
    Ok(resources
        .read::<names::Registrations>()
        .map_err(|_| "publication names not installed")?
        .seen)
}
