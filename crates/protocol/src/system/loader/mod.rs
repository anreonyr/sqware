//! Compatibility facade for Loader client API.
pub mod client;
pub mod frame;
pub mod grant;
pub mod marks;
pub mod rpc;
pub use system_client::loader::{Built, Face, DIR, Fail, Grant, grant_of, REGISTRY};
