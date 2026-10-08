mod client;
use system_api::identity::frame::vocab::*;
use system_api::identity::frame::{self, Fail, Reply, Wire};
use system_api::identity::grant::Grant;
use system_api::operator::Path;

const DIR: &Path = Path::new(system_api::identity::DIR);

pub use client::{CallError, Face, Installer, Organization, Query, SelfOps, TaskQuery};

mod discovery;
pub use discovery::authority;
