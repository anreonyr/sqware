//! Compatibility facade for the Identity API and client.
pub mod client;
pub mod frame;
pub mod grant;
pub mod limits;
pub mod marks;
pub mod rpc;
pub use system_client::identity::*;
