//! One authority owns the principal tree, coalitions, membership, and atomic task identity snapshots.
use alloc::vec::Vec;
use env::PieToken;
use env::TaskId;
use system_api::identity::CoalitionId;
use system_api::identity::Fail;
use system_api::identity::PrincipalId;
use system_api::identity::{
    IdentityName, IdentityObject, InstallId, InstallSpec, InstallState, OperatorEntryRef,
};

mod coalition;
mod coalition_node;
mod limits;
mod membership;
mod principal;
mod reference;
mod roster;

use coalition_node::CoalitionNode;
use principal::PrincipalNode;
pub use roster::Anchor;
use roster::BindingRow;

pub struct IdentityBook {
    authority: TaskId,
    installer: TaskId,
    principals: Vec<PrincipalNode>,
    coalitions: Vec<CoalitionNode>,
    memberships: Vec<(PrincipalId, CoalitionId)>,
    bindings: Vec<BindingRow>,
    revision: u64,
    installations: Vec<InstallRecord>,
    install_origins: Vec<InstallOrigin>,
    references: Vec<IdentityReference>,
}

impl IdentityBook {
    pub fn new(authority: TaskId, installer: TaskId) -> Result<Self, Fail> {
        let mut principals = Vec::new();
        principals.try_reserve(1).map_err(|_| Fail::Full)?;
        principals.push(PrincipalNode { parent: None });
        Ok(Self {
            authority,
            installer,
            principals,
            coalitions: Vec::new(),
            memberships: Vec::new(),
            bindings: Vec::new(),
            revision: 0,
            installations: Vec::new(),
            install_origins: Vec::new(),
            references: Vec::new(),
        })
    }
}

const MAX_INSTALLATIONS: usize = 256;
const MAX_INSTALL_ORIGINS: usize = 64;
struct InstallOrigin {
    endpoint: PieToken,
    caller: TaskId,
    high: u64,
}
struct IdentityReference {
    object: IdentityObject,
    name: IdentityName,
    entry: OperatorEntryRef,
}
struct InstallRecord {
    endpoint: PieToken,
    caller: TaskId,
    id: InstallId,
    spec: Option<InstallSpec>,
    state: InstallState,
    failure: Option<Fail>,
}

#[cfg(test)]
mod tests;

pub use membership::Membership;
pub use reference::{Association, ReferenceQuery};
pub use roster::{BindingRequest, InstallAction, InstallationRequest, Selection};
