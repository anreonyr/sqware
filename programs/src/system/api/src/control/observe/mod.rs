//! Public supervision observation contract.
//!
//! A page position selects one declared supervised target. The reply carries
//! presence, the explicit end marker, the observed lifecycle state and the
//! target's run intent. It never carries a task or team identity.

pub mod call;
pub mod reply;
pub mod request;

pub use call::Call;
pub use reply::Reply;
pub use request::Request;
