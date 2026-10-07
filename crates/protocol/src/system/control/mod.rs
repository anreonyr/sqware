//! Compatibility facade for Control API and clients.
pub mod client;
pub mod frame;
pub mod marks;
pub mod grant;
pub mod rpc;
pub mod publication;
pub mod account;
pub use system_client::control::*;
