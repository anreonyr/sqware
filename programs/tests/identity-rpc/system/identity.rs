pub use system_api::identity::{BACK, Fail, Grant, Reply, Wire};
pub const DIR: &crate::common::path::Path = &crate::common::path::Path::new("/svc/sys/identity");

pub mod frame { pub use system_api::identity::Request; }
#[path = "../../../../crates/protocol/src/system/identity/rpc.rs"]
pub mod rpc;
pub mod client;
