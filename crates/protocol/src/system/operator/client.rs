//! Compatibility reexports for system-client Operator clients.
pub use system_client::operator::client::*;
pub mod pane { pub use system_client::operator::client::pane::*; }
pub mod tile { pub use system_client::operator::client::tile::*; }
pub mod watch { pub use system_client::operator::client::watch::*; }
