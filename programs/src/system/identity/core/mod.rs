//! One authority owns the principal tree, coalitions, membership, and atomic task identity snapshots.
use alloc::vec::Vec;
use env::TaskId;
use protocol::system::identity::{CoalitionId, Fail, PrincipalId};

mod coalition;
mod membership;
mod principal;
mod roster;

use coalition::CoalitionNode;
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
}

impl IdentityBook {
    pub fn new(authority: TaskId, installer: TaskId) -> Result<Self, Fail> {
        let mut principals = Vec::new();
        principals.try_reserve(1).map_err(|_| Fail::Full)?;
        principals.push(PrincipalNode { parent: None });
        Ok(Self {
            authority, installer, principals, coalitions: Vec::new(),
            memberships: Vec::new(), bindings: Vec::new(), revision: 0,
        })
    }
}

#[cfg(test)]
mod tests;

pub use membership::Membership;
pub use roster::{BindingRequest, Selection};
