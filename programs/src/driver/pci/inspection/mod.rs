//! Bounded read-only configuration snapshots. No ECAM capability is exported.

pub mod call;
pub mod enumerate;
pub mod page;
pub mod scan;

pub use call::Call;
pub use page::Page;
pub use scan::Scan;

pub const ENTRY: env::Mark = super::api::CONFIG;
pub const ENTRY_PATH: &str = "pci/config";
pub const BACK: env::Mark = env::Mark::of("pci/config-back");
pub const CAPACITY: u8 = 1;
pub const WORK_LIMIT: u8 = 2;
pub const EOF: u8 = 3;
