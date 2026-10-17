use super::{AddressRange, DomainId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainRange {
    pub domain: DomainId,
    pub range: AddressRange,
}
