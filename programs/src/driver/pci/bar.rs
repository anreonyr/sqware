use execution::memory::Reference;
use resource::raw::Capability;
use super::placement::Placement;

pub(super) struct Bar {
    pub index: u8,
    pub local: Option<Capability>,
    /// Parent window retained only while the child root is being established.
    pub reference: Option<Reference>,
    pub placement: Option<Placement>,
}
