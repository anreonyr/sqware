use env::Mark;
use system_api::operator::Path;

/// Relative interface path under the SERVICE namespace.
pub const ENTRY_PATH: &Path = Path::new("fixturetarget/entry");
pub const ENTRY_NAME: &str = "entry";
pub const ENTRY_MARK: Mark = Mark::of("fixturetarget/entry");
pub const BACK: Mark = Mark::of("fixturetarget/back");
pub const ASK: u8 = 1;
pub const BEAT: u8 = 2;
