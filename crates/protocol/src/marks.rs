//! Protocol-wide validation belongs at the protocol composition boundary.
use crate::common::marks::{Definition, conflict};
pub(crate) const GROUPS: &[&[Definition]] = &[
    crate::system::control::marks::DECLARATIONS,
    &crate::system::control::Grant::DECLARATIONS,
    &crate::system::identity::marks::DECLARATIONS,
    &crate::system::identity::Grant::DECLARATIONS,
    crate::system::operator::marks::DECLARATIONS,
    &crate::system::operator::Grant::DECLARATIONS,
    crate::system::loader::REGISTRY,
    crate::driver::marks::DECLARATIONS,
    crate::service::hub::marks::DECLARATIONS,
    &crate::service::hub::Grant::DECLARATIONS,
    crate::service::terminal::marks::DECLARATIONS,
];
const _: () = assert!(conflict(GROUPS).is_none(), "protocol mark collision");
