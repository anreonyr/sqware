use resource::raw::Capability;
use super::placement::Placement;

pub(super) struct Bar {
    pub index: u8,
    pub local: Option<Capability>,
    pub placement: Option<Placement>,
}
