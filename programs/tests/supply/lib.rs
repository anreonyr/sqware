extern crate alloc;

#[path = "../../src/system/supply/derivation.rs"]
mod derivation;

#[path = "../../src/system/supply/lease.rs"]
mod lease;

#[path = "../../src/system/supply/mod.rs"]
pub mod production_source;

use env::{Mark, Permission, TaskId};
use lease::{Origin, PrivateRoot, Provision, ProvisionError, RootAssignment, RootRequest};
use system_api::supply::{Fail, Key};

pub struct LeaseRequest {
    pub id: system_api::supply::LeaseId,
    pub origin: Origin,
    pub source: Key,
    pub resource: Key,
    pub target: TaskId,
    pub slot: Mark,
    pub permission: Permission,
    pub owner: TaskId,
}

impl LeaseRequest {
    pub fn derive(&self, assignment: RootAssignment) -> Result<PrivateRoot, Fail> {
        PrivateRoot::derive(
            assignment,
            RootRequest {
                id: self.id,
                origin: self.origin,
                owner: self.owner,
                permission: self.permission,
            },
        )
    }
}

pub trait Resource {
    fn key(&self) -> Key;
    fn authorize(&self, request: &LeaseRequest) -> bool;
    fn acquire(&mut self, request: &LeaseRequest) -> Result<Provision, ProvisionError>;
    fn shutdown(&mut self) -> Result<(), Fail> {
        Ok(())
    }
}

#[path = "../../src/system/supply/pole.rs"]
pub mod production_pole;

pub mod provider_test {
    pub use super::LeaseRequest;
    pub use super::production_pole as pole;
}

#[cfg(test)]
mod tests;
