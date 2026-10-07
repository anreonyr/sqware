//! Compatibility facade for the Control account client.
pub use system_client::control::account::Client;
pub use system_api::control::account::Request;
pub mod frame { pub use system_api::control::account::frame::*; }
pub use system_api::control::marks::{ACCOUNT_BACK as BACK, ACCOUNT_ENTRY as ENTRY};
pub use system_api::control::account::DIR;
