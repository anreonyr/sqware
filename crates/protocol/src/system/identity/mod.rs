//! One authority owns lineage, membership qualifications and effective task identities.
//! IDs survive transport, not authority restart. Subjects are bounded snapshots, never credentials.
pub mod client;
pub mod frame;
pub mod grant;
pub mod limits;

pub use client::{CallError, Installer, Organization, Query, SelfOps, TaskQuery};
pub use frame::{BACK, Fail, Reply, Wire};
pub use grant::{Grant, Mount, grant_of};
pub use frame::vocab::*;

pub const DIR: &crate::common::path::Path = crate::common::path::Path::new(frame::DIR);
