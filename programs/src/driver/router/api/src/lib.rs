#![no_std]

pub mod frame;
pub mod marks;
pub const REGISTRY: &[&[env::marks::Definition]] = &[marks::DECLARATIONS];
const _: () = assert!(env::marks::conflict(REGISTRY).is_none());

pub use frame::Fail;
pub use marks::{ENTRY_MARK, LANE, LINE_BACK, LINE_MARK};
pub use system_api::operator::path::Path;

pub const SVC: &Path = Path::new("svc");
pub const DIR: &str = "drv";
pub const ROAD: &Path = Path::new("svc/drv");
