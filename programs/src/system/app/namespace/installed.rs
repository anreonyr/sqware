//! Namespace installation receipt retained across prepare and retire.

use alloc::vec::Vec;
use env::Wait;
use system_api::operator::{Binding, Entry, GrantId};
use system_client::operator::GrantInfo;

/// Retained by Assembly across prepare/retire so grants and bindings can be retried.
pub struct Installed {
    pub anchor: Entry,
    pub authority: Option<GrantInfo>,
    /// Kept before the grant request so a lost reply can still cancel an unseen/pending grant.
    pub grant: Option<GrantId>,
    pub bindings: Vec<Binding>,
    pub entries: Vec<Entry>,
    pub wait: Wait,
}
