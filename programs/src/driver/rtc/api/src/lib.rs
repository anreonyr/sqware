#![no_std]

use system_api::operator::Path;

/// Relative interface path under the SERVICE namespace.
pub const ENTRY_PATH: &Path = Path::new("rtc/entry");
pub const ENTRY_NAME: &str = "entry";
